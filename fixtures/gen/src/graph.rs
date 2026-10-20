//! Synthetic scale graph for performance work (not committed output).
//!
//! `cargo run --release -p openstance-fixtures --bin gen-graph -- --people 20000
//! --interactions 500000 --seed 42 --out target/synth/heavy` writes a
//! headers-only mbox, a LinkedIn `Connections.csv` (up to 5k rows) and a vcf
//! with roughly 30% overlap, from a deterministic PRNG. Interactions per
//! person are skewed (a few heavy contacts, a long tail), timestamps spread
//! over six years with recency skew, and 3% of people are bulk senders.

use std::io::{BufWriter, Write};
use std::path::Path;

use crate::{from_line_date, linkedin_date, rfc2822, ts, OWNER_EMAIL, OWNER_NAME};

/// Generator parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphParams {
    /// People in the graph (excluding the owner).
    pub people: u32,
    /// Email interactions in the mbox.
    pub interactions: u32,
    /// PRNG seed.
    pub seed: u64,
}

impl Default for GraphParams {
    fn default() -> Self {
        GraphParams {
            people: 20_000,
            interactions: 500_000,
            seed: 42,
        }
    }
}

/// splitmix64: tiny, deterministic, good enough for fixtures.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Seeded generator.
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}

const FIRST: &[&str] = &[
    "Avery", "Bram", "Cleo", "Dario", "Esme", "Faro", "Gita", "Hollis", "Ilka", "Joss", "Kaia",
    "Lumi", "Mateo", "Nell", "Orrin", "Pavi", "Quill", "Rhea", "Soren", "Tamsin", "Uma", "Vero",
    "Wren", "Xavi", "Yara", "Zeno",
];
const LAST: &[&str] = &[
    "Ashgrove",
    "Brindle",
    "Copperfield",
    "Dunmore",
    "Elsworth",
    "Fairweather",
    "Glenholm",
    "Harrowgate",
    "Ivybridge",
    "Juniper",
    "Kestrel",
    "Larkspur",
    "Mossley",
    "Northcote",
    "Oakhurst",
    "Pennant",
    "Quarrington",
    "Ravenscar",
    "Silverdale",
    "Thornbury",
    "Underhill",
    "Valemont",
    "Westbrook",
    "Yarrowby",
];
const COMPANIES: &[&str] = &[
    "Northwind",
    "Contoso",
    "Fabrikam",
    "Tailspin",
    "Brightwater",
    "Halcyon",
    "Woodgrove",
    "Litware",
    "Proseware",
    "Adatum",
    "Lucerne",
    "Margie",
    "Trey",
    "Wingtip",
    "Alpine",
    "Coho",
];

/// A synthetic person.
#[derive(Debug, Clone)]
pub struct SynthPerson {
    /// Display name.
    pub name: String,
    /// Work email.
    pub email: String,
    /// Company.
    pub company: &'static str,
    /// LinkedIn slug.
    pub slug: String,
    /// Newsletter-like sender.
    pub bulk: bool,
}

/// The deterministic population for `p`.
pub fn population(p: &GraphParams) -> Vec<SynthPerson> {
    let mut rng = Rng::new(p.seed ^ 0xA5A5_A5A5);
    (0..p.people)
        .map(|i| {
            let f = FIRST[rng.below(FIRST.len() as u64) as usize];
            let l = LAST[rng.below(LAST.len() as u64) as usize];
            let company = COMPANIES[rng.below(COMPANIES.len() as u64) as usize];
            let bulk = rng.below(100) < 3;
            let domain = format!("{}.example", company.to_lowercase());
            let email = if bulk {
                format!("news{i}@{domain}")
            } else {
                format!("{}.{}{}@{}", f.to_lowercase(), l.to_lowercase(), i, domain)
            };
            SynthPerson {
                name: format!("{f} {l}"),
                email,
                company,
                slug: format!("{}-{}-{i}", f.to_lowercase(), l.to_lowercase()),
                bulk,
            }
        })
        .collect()
}

/// Write the synthetic sources into `out`. Returns the number of messages.
pub fn write_graph(p: &GraphParams, out: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(out)?;
    let people = population(p);
    let mut rng = Rng::new(p.seed);
    let now = ts(2026, 10, 1, 0, 0);
    let six_years = 6 * 365 * 86_400_i64;

    let mut mbox = BufWriter::new(std::fs::File::create(out.join("heavy.mbox"))?);
    let n = people.len().max(1) as f64;
    for k in 0..p.interactions {
        // Skewed counterpart choice: u^3 concentrates on low indices.
        let u = rng.unit();
        let idx = ((u * u * u) * n) as usize % people.len().max(1);
        let who = &people[idx];
        let age = (rng.unit() * rng.unit() * six_years as f64) as i64;
        let at = now - age;
        let outbound = !who.bulk && rng.below(2) == 0;
        let (from_n, from_e, to_n, to_e) = if outbound {
            (
                OWNER_NAME,
                OWNER_EMAIL,
                who.name.as_str(),
                who.email.as_str(),
            )
        } else {
            (
                who.name.as_str(),
                who.email.as_str(),
                OWNER_NAME,
                OWNER_EMAIL,
            )
        };
        writeln!(mbox, "From {from_e} {}", from_line_date(at))?;
        writeln!(
            mbox,
            "X-GM-THRID: {}",
            2_000_000_000_000_000_u64 + (idx as u64) * 64 + rng.below(8)
        )?;
        writeln!(
            mbox,
            "X-Gmail-Labels: {}",
            if outbound { "Sent" } else { "Inbox" }
        )?;
        writeln!(
            mbox,
            "Message-ID: <syn{k}.{}@{}>",
            p.seed,
            from_e.rsplit('@').next().unwrap_or("example.com")
        )?;
        writeln!(mbox, "Date: {}", rfc2822(at))?;
        writeln!(mbox, "From: {from_n} <{from_e}>")?;
        writeln!(mbox, "To: {to_n} <{to_e}>")?;
        if !who.bulk && rng.below(5) == 0 {
            let other = &people[rng.below(people.len() as u64) as usize];
            if !other.bulk {
                writeln!(mbox, "Cc: {} <{}>", other.name, other.email)?;
            }
        }
        writeln!(mbox, "Subject: {} update {}", who.company, k % 997)?;
        if who.bulk {
            writeln!(
                mbox,
                "List-Unsubscribe: <https://{}.example/u>",
                who.company.to_lowercase()
            )?;
        }
        writeln!(mbox)?;
        writeln!(mbox, "synthetic body")?;
        writeln!(mbox)?;
    }
    mbox.flush()?;

    let mut csv = BufWriter::new(std::fs::File::create(out.join("Connections.csv"))?);
    writeln!(
        csv,
        "First Name,Last Name,URL,Email Address,Company,Position,Connected On"
    )?;
    for (i, person) in people.iter().filter(|x| !x.bulk).take(5000).enumerate() {
        let (f, l) = person.name.split_once(' ').unwrap_or((&person.name, ""));
        let email = if i % 3 == 0 {
            person.email.as_str()
        } else {
            ""
        };
        let at = now - (rng.unit() * six_years as f64) as i64;
        writeln!(
            csv,
            "{f},{l},https://www.linkedin.com/in/{},{email},{},Engineer,{}",
            person.slug,
            person.company,
            linkedin_date(at)
        )?;
    }
    csv.flush()?;

    let mut vcf = BufWriter::new(std::fs::File::create(out.join("contacts.vcf"))?);
    for (i, person) in people
        .iter()
        .enumerate()
        .filter(|(i, x)| !x.bulk && i % 10 < 3)
    {
        write!(
            vcf,
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:{}\r\nORG:{}\r\nEMAIL:{}\r\nTEL:+1 201 555 {:04}\r\nEND:VCARD\r\n",
            person.name,
            person.company,
            person.email,
            100 + i % 100
        )?;
    }
    vcf.flush()?;
    Ok(u64::from(p.interactions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_graph_is_deterministic_and_complete() {
        let p = GraphParams {
            people: 200,
            interactions: 2000,
            seed: 7,
        };
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(write_graph(&p, a.path()).unwrap(), 2000);
        write_graph(&p, b.path()).unwrap();
        for f in ["heavy.mbox", "Connections.csv", "contacts.vcf"] {
            let x = std::fs::read(a.path().join(f)).unwrap();
            let y = std::fs::read(b.path().join(f)).unwrap();
            assert_eq!(x, y, "{f} differs between runs");
        }
        let mbox = std::fs::read_to_string(a.path().join("heavy.mbox")).unwrap();
        assert_eq!(
            mbox.lines().filter(|l| l.starts_with("From ")).count(),
            2000
        );
        let people = population(&p);
        let bulk = people.iter().filter(|x| x.bulk).count();
        assert!(bulk > 0 && bulk < 20, "about 3% bulk, got {bulk}");
    }

    #[test]
    fn rng_is_reproducible() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(1);
        assert_eq!(
            (0..5).map(|_| a.next_u64()).collect::<Vec<_>>(),
            (0..5).map(|_| b.next_u64()).collect::<Vec<_>>()
        );
        assert!((0..100).map(|_| a.unit()).all(|u| (0.0..1.0).contains(&u)));
    }
}
