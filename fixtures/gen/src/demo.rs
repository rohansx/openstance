//! The demo dataset for the 60-second demo and the README (P2-DOC-01).
//!
//! `cargo run -p openstance-fixtures --bin gen-demo` writes
//! `fixtures/demo/{takeout.mbox, Connections.csv, contacts.vcf}`: about 600
//! fictional people around a fictional owner (`you@example.com`), built so
//! the demo queries in `docs/plan/phase-2.md` give clean, believable
//! answers. Every person and company is invented, every domain is under
//! `.example` and every phone number is in the fictional `555-01xx` range.
//!
//! The population is a hand-written cast ([`CAST`]: the people the demo
//! talks about) plus a deterministic background (a few hundred people at
//! about 40 companies, with a skewed contact distribution, colleagues cc'd
//! together, people you lost touch with, one-off cold inbound, newsletters).
//! The tie structure behind `how-connected "Dana Whitfield"` and the three
//! flagged merges in the review queue are planted on purpose; everything
//! else is drawn from a seeded PRNG, so the files are byte-identical on
//! every run. A test fails if the committed files drift from the generator.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::graph::Rng;
use crate::{from_line_date, linkedin_date, rfc2822, ts, vcard_rev};

/// The owner's display name.
pub const OWNER_NAME: &str = "Jordan Avery";
/// The owner's address (`openstance init --me you@example.com`).
pub const OWNER_EMAIL: &str = "you@example.com";

/// The three export files, in import order.
pub const EXPORTS: &[&str] = &["takeout.mbox", "Connections.csv", "contacts.vcf"];

/// "Today" for the demo (2026-10-15T09:00:00Z).
pub fn demo_now() -> i64 {
    ts(2026, 10, 15, 9, 0)
}

/// People the review queue must flag (and nobody else).
pub const FLAGGED: &[&str] = &["Elif Kaya", "Noah Lindqvist", "Marco Bianchi"];
/// Founders at fintech companies (the 0:15 demo query).
pub const FINTECH_FOUNDERS: &[&str] = &[
    "Maya Chen",
    "Arjun Mehta",
    "Sofia Brandt",
    "Lucas Moreau",
    "Nadia Petrova",
];
/// Everyone the owner knows at Lumen Pay (the 0:45 demo question).
pub const LUMEN_PAY: &[&str] = &[
    "Dana Whitfield",
    "Theo Marsh",
    "Ines Duarte",
    "Kwame Asante",
    "Rosa Lindgren",
];
/// The three mutuals behind `how-connected "Dana Whitfield"`.
pub const DANA_MUTUALS: &[&str] = &["Theo Marsh", "Maya Chen", "Rafael Ortiz"];
/// People who build developer tools (the Phase 2 semantic example).
pub const DEVTOOLS: &[&str] = &[
    "Elif Kaya",
    "Priya Natarajan",
    "Ben Adler",
    "Grace Liu",
    "Owen Gallagher",
];

// ---------------------------------------------------------------------------
// Companies
// ---------------------------------------------------------------------------

/// A fictional company.
#[derive(Debug, Clone, Copy)]
pub struct Company {
    /// Display name.
    pub name: &'static str,
    /// Mail domain (under `.example`).
    pub domain: &'static str,
    /// Industry, used to pick subjects and titles.
    pub industry: &'static str,
    /// Home city.
    pub city: &'static str,
    /// Region / state for vCard `ADR`.
    pub region: &'static str,
    /// Country for vCard `ADR`.
    pub country: &'static str,
}

const fn co(
    name: &'static str,
    domain: &'static str,
    industry: &'static str,
    city: &'static str,
    region: &'static str,
    country: &'static str,
) -> Company {
    Company {
        name,
        domain,
        industry,
        city,
        region,
        country,
    }
}

/// Every company in the demo graph. Names are invented for this dataset.
pub const COMPANIES: &[Company] = &[
    co(
        "Lumen Pay",
        "lumenpay.example",
        "fintech",
        "San Francisco",
        "CA",
        "USA",
    ),
    co(
        "Ledgerline",
        "ledgerline.example",
        "fintech",
        "New York",
        "NY",
        "USA",
    ),
    co(
        "Tallyfin",
        "tallyfin.example",
        "fintech",
        "London",
        "",
        "UK",
    ),
    co(
        "Fernpay",
        "fernpay.example",
        "fintech",
        "Berlin",
        "",
        "Germany",
    ),
    co(
        "Brightvault",
        "brightvault.example",
        "fintech",
        "Singapore",
        "",
        "Singapore",
    ),
    co(
        "Quaystone",
        "quaystone.example",
        "fintech",
        "Toronto",
        "ON",
        "Canada",
    ),
    co(
        "Coinwell",
        "coinwell.example",
        "crypto",
        "Lisbon",
        "",
        "Portugal",
    ),
    co(
        "Stackforge",
        "stackforge.example",
        "devtools",
        "San Francisco",
        "CA",
        "USA",
    ),
    co(
        "Hexwire",
        "hexwire.example",
        "devtools",
        "Seattle",
        "WA",
        "USA",
    ),
    co(
        "Quillstack",
        "quillstack.example",
        "devtools",
        "Berlin",
        "",
        "Germany",
    ),
    co(
        "Driftless",
        "driftless.example",
        "devtools",
        "Austin",
        "TX",
        "USA",
    ),
    co(
        "Patchwork Labs",
        "patchworklabs.example",
        "devtools",
        "Toronto",
        "ON",
        "Canada",
    ),
    co(
        "Juniper Health",
        "juniperhealth.example",
        "healthtech",
        "Boston",
        "MA",
        "USA",
    ),
    co(
        "Carebridge",
        "carebridge.example",
        "healthtech",
        "Chicago",
        "IL",
        "USA",
    ),
    co(
        "Greenloop",
        "greenloop.example",
        "climate",
        "Amsterdam",
        "",
        "Netherlands",
    ),
    co(
        "Solstice Grid",
        "solsticegrid.example",
        "climate",
        "Austin",
        "TX",
        "USA",
    ),
    co(
        "Orbital Robotics",
        "orbitalrobotics.example",
        "robotics",
        "Tokyo",
        "",
        "Japan",
    ),
    co(
        "Tinkerworks",
        "tinkerworks.example",
        "hardware",
        "Seattle",
        "WA",
        "USA",
    ),
    co(
        "Kitebox",
        "kitebox.example",
        "consumer",
        "Los Angeles",
        "CA",
        "USA",
    ),
    co(
        "Marigold Goods",
        "marigoldgoods.example",
        "consumer",
        "New York",
        "NY",
        "USA",
    ),
    co(
        "Ridgeway",
        "ridgeway.example",
        "saas",
        "Chicago",
        "IL",
        "USA",
    ),
    co(
        "Vellum Desk",
        "vellumdesk.example",
        "saas",
        "Dublin",
        "",
        "Ireland",
    ),
    co(
        "Corvid Analytics",
        "corvidanalytics.example",
        "saas",
        "London",
        "",
        "UK",
    ),
    co(
        "Tessellate",
        "tessellate.example",
        "saas",
        "Paris",
        "",
        "France",
    ),
    co(
        "Keystone Shield",
        "keystoneshield.example",
        "security",
        "Tel Aviv",
        "",
        "Israel",
    ),
    co(
        "Sentry Pine",
        "sentrypine.example",
        "security",
        "Boston",
        "MA",
        "USA",
    ),
    co(
        "Brightpath Learning",
        "brightpath.example",
        "edtech",
        "Sydney",
        "NSW",
        "Australia",
    ),
    co(
        "Pixelmoor",
        "pixelmoor.example",
        "gaming",
        "Tokyo",
        "",
        "Japan",
    ),
    co(
        "Meridian Ventures",
        "meridianvc.example",
        "vc",
        "San Francisco",
        "CA",
        "USA",
    ),
    co(
        "Northstar Seed",
        "northstarseed.example",
        "vc",
        "New York",
        "NY",
        "USA",
    ),
    co(
        "Harbor Lane Capital",
        "harborlane.example",
        "vc",
        "London",
        "",
        "UK",
    ),
    co(
        "Halden & Rowe",
        "haldenrowe.example",
        "legal",
        "New York",
        "NY",
        "USA",
    ),
    co(
        "Mosaic Studio",
        "mosaicstudio.example",
        "design",
        "Paris",
        "",
        "France",
    ),
    co(
        "Arclight Media",
        "arclightmedia.example",
        "media",
        "Los Angeles",
        "CA",
        "USA",
    ),
    co(
        "Granite Logistics",
        "granitelogistics.example",
        "logistics",
        "Chicago",
        "IL",
        "USA",
    ),
    co(
        "Bluefin Bio",
        "bluefinbio.example",
        "biotech",
        "Boston",
        "MA",
        "USA",
    ),
    co(
        "Openfield Labs",
        "openfieldlabs.example",
        "ai",
        "San Francisco",
        "CA",
        "USA",
    ),
    co(
        "Wayfarer Travel",
        "wayfarer.example",
        "consumer",
        "Barcelona",
        "",
        "Spain",
    ),
];

#[cfg(test)]
fn company(name: &str) -> &'static Company {
    COMPANIES
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("unknown demo company {name}"))
}

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

/// Where a person shows up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sources {
    /// Has mail in the Takeout mbox.
    pub mbox: bool,
    /// Has a LinkedIn connection row.
    pub linkedin: bool,
    /// The LinkedIn row carries the email address.
    pub linkedin_email: bool,
    /// Has a vCard.
    pub vcf: bool,
}

/// How much mail a background person exchanges with the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Close, frequent, two-way, recent.
    Close,
    /// Regular contact over the last year or two.
    Regular,
    /// A few threads.
    Occasional,
    /// Strong once, nothing for two to four years.
    LostTouch,
    /// One cold or one-off message.
    OneOff,
    /// Hand-written threads only (cast members).
    Scripted,
}

/// One fictional person.
#[derive(Debug, Clone)]
pub struct DemoPerson {
    /// First name.
    pub first: String,
    /// Last name.
    pub last: String,
    /// Index into [`COMPANIES`].
    pub company: usize,
    /// Job title.
    pub title: String,
    /// Work email.
    pub email: String,
    /// LinkedIn vanity slug.
    pub slug: String,
    /// Phone (E.164) on the vCard, if any.
    pub phone: Option<String>,
    /// Sources this person appears in.
    pub sources: Sources,
    /// Contact pattern.
    pub tier: Tier,
    /// LinkedIn `Connected On`.
    pub connected: i64,
}

impl DemoPerson {
    /// `First Last`.
    pub fn name(&self) -> String {
        format!("{} {}", self.first, self.last)
    }

    fn company(&self) -> &'static Company {
        &COMPANIES[self.company]
    }
}

/// One hand-written cast member: (first, last, company, title, sources).
struct CastEntry {
    first: &'static str,
    last: &'static str,
    company: &'static str,
    title: &'static str,
    /// Local part of the work email.
    local: &'static str,
    sources: Sources,
    connected: (i64, i64, i64),
}

const fn src(mbox: bool, linkedin: bool, linkedin_email: bool, vcf: bool) -> Sources {
    Sources {
        mbox,
        linkedin,
        linkedin_email,
        vcf,
    }
}

const fn cast(
    first: &'static str,
    last: &'static str,
    company: &'static str,
    title: &'static str,
    local: &'static str,
    sources: Sources,
    connected: (i64, i64, i64),
) -> CastEntry {
    CastEntry {
        first,
        last,
        company,
        title,
        local,
        sources,
        connected,
    }
}

/// The people the demo talks about. Each one's mail is scripted in
/// [`scripted_threads`].
const CAST: &[CastEntry] = &[
    // The warm fintech founder from `who "Maya Chen"`.
    cast(
        "Maya",
        "Chen",
        "Ledgerline",
        "Co-founder & CEO",
        "maya",
        src(true, true, true, true),
        (2021, 4, 12),
    ),
    // The how-connected target: in the graph, but only cc'd.
    cast(
        "Dana",
        "Whitfield",
        "Lumen Pay",
        "CTO",
        "dana",
        src(true, true, true, false),
        (2023, 9, 4),
    ),
    cast(
        "Theo",
        "Marsh",
        "Lumen Pay",
        "Head of Partnerships",
        "theo",
        src(true, true, true, true),
        (2022, 2, 17),
    ),
    cast(
        "Ines",
        "Duarte",
        "Lumen Pay",
        "Staff Engineer",
        "ines",
        src(true, false, false, true),
        (2024, 1, 1),
    ),
    cast(
        "Kwame",
        "Asante",
        "Lumen Pay",
        "Product Manager",
        "kwame",
        src(true, true, true, false),
        (2025, 3, 3),
    ),
    cast(
        "Rosa",
        "Lindgren",
        "Lumen Pay",
        "VP Finance",
        "rosa",
        src(false, true, false, false),
        (2024, 6, 21),
    ),
    cast(
        "Rafael",
        "Ortiz",
        "Meridian Ventures",
        "Partner",
        "rafael",
        src(true, true, true, true),
        (2020, 11, 2),
    ),
    cast(
        "Aisha",
        "Bello",
        "Meridian Ventures",
        "Talent Partner",
        "aisha",
        src(true, true, true, false),
        (2023, 5, 9),
    ),
    // Other fintech founders.
    cast(
        "Arjun",
        "Mehta",
        "Tallyfin",
        "Founder & CEO",
        "arjun",
        src(true, true, true, true),
        (2022, 8, 30),
    ),
    cast(
        "Sofia",
        "Brandt",
        "Fernpay",
        "Co-founder & CTO",
        "sofia",
        src(true, true, true, false),
        (2023, 3, 14),
    ),
    cast(
        "Lucas",
        "Moreau",
        "Brightvault",
        "Founder",
        "lucas",
        src(true, true, true, false),
        (2024, 10, 8),
    ),
    cast(
        "Nadia",
        "Petrova",
        "Quaystone",
        "Co-founder & COO",
        "nadia",
        src(true, false, false, true),
        (2024, 1, 1),
    ),
    // Founders outside fintech (they must not answer the fintech query).
    cast(
        "Hana",
        "Sato",
        "Coinwell",
        "Co-founder",
        "hana",
        src(true, true, true, false),
        (2022, 12, 1),
    ),
    cast(
        "Owen",
        "Gallagher",
        "Stackforge",
        "Founder & CEO",
        "owen",
        src(true, true, true, true),
        (2021, 7, 19),
    ),
    cast(
        "Leila",
        "Haddad",
        "Greenloop",
        "Co-founder",
        "leila",
        src(true, true, true, false),
        (2023, 10, 10),
    ),
    cast(
        "Tomas",
        "Rivera",
        "Juniper Health",
        "Founder",
        "tomas",
        src(true, true, true, false),
        (2024, 4, 4),
    ),
    // Developer tools people.
    cast(
        "Elif",
        "Kaya",
        "Stackforge",
        "Developer Advocate",
        "elif",
        src(true, true, false, false),
        (2024, 2, 26),
    ),
    cast(
        "Priya",
        "Natarajan",
        "Hexwire",
        "API Platform Lead",
        "priya",
        src(true, true, true, false),
        (2023, 7, 7),
    ),
    cast(
        "Ben",
        "Adler",
        "Quillstack",
        "SDK Engineer",
        "ben",
        src(true, false, false, true),
        (2024, 1, 1),
    ),
    cast(
        "Grace",
        "Liu",
        "Driftless",
        "Head of Developer Relations",
        "grace",
        src(true, true, true, false),
        (2022, 5, 16),
    ),
    // Planted review cases.
    cast(
        "Noah",
        "Lindqvist",
        "Tallyfin",
        "Head of Growth",
        "noah",
        src(true, true, false, false),
        (2025, 1, 20),
    ),
    cast(
        "Marco",
        "Bianchi",
        "Kitebox",
        "Design Lead",
        "marco",
        src(true, false, false, false),
        (2024, 1, 1),
    ),
    // A close colleague you lost touch with.
    cast(
        "Hollis",
        "Grant",
        "Ridgeway",
        "VP Engineering",
        "hollis",
        src(true, true, true, true),
        (2019, 6, 3),
    ),
];

const FIRST: &[&str] = &[
    "Abena", "Adrian", "Aiko", "Alana", "Amir", "Anika", "Anton", "Aria", "Astrid", "Bastian",
    "Beatrice", "Bilal", "Bruno", "Camila", "Carmen", "Cedric", "Chiara", "Colette", "Daria",
    "Darius", "Delia", "Diego", "Dmitri", "Eamon", "Elena", "Emeka", "Esther", "Ezra", "Farah",
    "Felix", "Freya", "Gideon", "Greta", "Hamza", "Harriet", "Hiro", "Idris", "Imani", "Isla",
    "Ivan", "Jasper", "Jun", "Kaveh", "Keira", "Kenji", "Lars", "Laila", "Leon", "Lina", "Lorenzo",
    "Mabel", "Malik", "Mateo", "Mira", "Nikhil", "Nina", "Oskar", "Paloma", "Quentin", "Ravi",
    "Rhea", "Ruben", "Saanvi", "Selin", "Silas", "Soraya", "Stellan", "Talia", "Tariq", "Ulla",
    "Vera", "Viktor", "Wen", "Yusuf", "Zara", "Zoltan",
];

const LAST: &[&str] = &[
    "Abara",
    "Achterberg",
    "Alvarado",
    "Amundsen",
    "Arkwright",
    "Balogun",
    "Barrientos",
    "Beaumont",
    "Bergqvist",
    "Castellano",
    "Chakraborty",
    "Cormack",
    "Dalgaard",
    "Delacroix",
    "Demir",
    "Drummond",
    "Easterbrook",
    "Ekwueme",
    "Falkenberg",
    "Fitzroy",
    "Gallo",
    "Gutierrez",
    "Halvorsen",
    "Hartigan",
    "Ishikawa",
    "Iversen",
    "Jablonski",
    "Kapoor",
    "Kowalczyk",
    "Kuznetsova",
    "Laurent",
    "Lindahl",
    "Maddox",
    "Mbeki",
    "Montague",
    "Nakashima",
    "Nkemelu",
    "Oduya",
    "Okonjo",
    "Ostrowski",
    "Pacheco",
    "Prescott",
    "Quintero",
    "Radcliffe",
    "Rautio",
    "Saavedra",
    "Sandoval",
    "Schreiber",
    "Sorensen",
    "Takahashi",
    "Thackeray",
    "Valdivia",
    "Varga",
    "Vasquez",
    "Wainwright",
    "Whitlock",
    "Yamamoto",
    "Yilmaz",
    "Zielinski",
    "Zubiri",
];

/// Titles by kind of company.
fn title_pool(industry: &str) -> &'static [&'static str] {
    match industry {
        "vc" => &[
            "Principal",
            "Associate",
            "Venture Partner",
            "Platform Lead",
            "Analyst",
        ],
        "legal" => &["Associate", "Counsel", "Paralegal"],
        "design" => &[
            "Product Designer",
            "Design Director",
            "Brand Designer",
            "Studio Manager",
        ],
        "devtools" => &[
            "Software Engineer",
            "Senior Software Engineer",
            "Staff Engineer",
            "Engineering Manager",
            "Product Manager",
            "Developer Advocate",
            "Solutions Engineer",
            "Account Executive",
            "Technical Writer",
            "Product Designer",
        ],
        _ => &[
            "Software Engineer",
            "Senior Software Engineer",
            "Staff Engineer",
            "Engineering Manager",
            "Data Scientist",
            "Product Manager",
            "Senior Product Manager",
            "Product Designer",
            "Account Executive",
            "Sales Director",
            "Partnerships Manager",
            "Marketing Lead",
            "Operations Manager",
            "Chief of Staff",
            "Technical Recruiter",
            "Customer Success Lead",
            "Finance Lead",
        ],
    }
}

/// Fictional phone numbers: `+1 <area> 555 01xx`, unique per index.
fn phone(i: usize) -> String {
    const AREAS: &[u32] = &[201, 202, 212, 312, 415, 503, 512, 617, 646, 718, 206, 303];
    format!("+1{}55501{:02}", AREAS[(i / 100) % AREAS.len()], i % 100)
}

fn slug(first: &str, last: &str, salt: u64) -> String {
    format!(
        "{}-{}-{:x}",
        first.to_lowercase(),
        last.to_lowercase(),
        salt & 0xfff
    )
}

/// The full population: the cast first, then the background.
pub fn people() -> Vec<DemoPerson> {
    let mut rng = Rng::new(0xDE30_2026);
    let mut out = Vec::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut emails: BTreeSet<String> = BTreeSet::new();
    let mut phones = 0usize;
    for c in CAST {
        let company_idx = COMPANIES.iter().position(|x| x.name == c.company).unwrap();
        let p = DemoPerson {
            first: c.first.into(),
            last: c.last.into(),
            company: company_idx,
            title: c.title.into(),
            email: format!("{}@{}", c.local, COMPANIES[company_idx].domain),
            slug: slug(c.first, c.last, rng.next_u64()),
            phone: c.sources.vcf.then(|| {
                phones += 1;
                phone(phones)
            }),
            sources: c.sources,
            tier: Tier::Scripted,
            connected: ts(c.connected.0, c.connected.1, c.connected.2, 0, 0),
        };
        names.insert(p.name());
        emails.insert(p.email.clone());
        out.push(p);
    }

    // Background: unique names, and no surname twice at one company (so the
    // near-name rule never fires).
    let mut surname_at: BTreeSet<(usize, &str)> = BTreeSet::new();
    let mut founders_at: BTreeSet<usize> = COMPANIES
        .iter()
        .enumerate()
        .filter(|(_, c)| c.industry == "fintech")
        .map(|(i, _)| i)
        .collect();
    for c in CAST {
        founders_at.insert(company_index(c.company));
    }
    const BACKGROUND: usize = 582;
    while out.len() < CAST.len() + BACKGROUND {
        let first = FIRST[rng.below(FIRST.len() as u64) as usize];
        let last = LAST[rng.below(LAST.len() as u64) as usize];
        let name = format!("{first} {last}");
        // Skewed company choice: a few big employers, a long tail.
        let u = rng.unit();
        // Lumen Pay (index 0) is cast-only: you know five people there.
        let ci =
            1 + (7 + ((u * u) * (COMPANIES.len() - 1) as f64) as usize) % (COMPANIES.len() - 1);
        // Near-identical names (`Aria Yamamoto` / `Daria Yamamoto`) would
        // land in the review queue as Jaro-Winkler suggestions.
        let key = name.to_lowercase();
        if names.contains(&name)
            || surname_at.contains(&(ci, last))
            || names
                .iter()
                .any(|n| jaro_winkler(&n.to_lowercase(), &key) >= 0.90)
        {
            continue;
        }
        let c = &COMPANIES[ci];
        let title = if !founders_at.contains(&ci)
            && c.industry != "vc"
            && c.industry != "legal"
            && ci % 3 == 0
        {
            founders_at.insert(ci);
            if rng.below(2) == 0 {
                "Founder & CEO"
            } else {
                "Co-founder"
            }
        } else {
            let pool = title_pool(c.industry);
            pool[rng.below(pool.len() as u64) as usize]
        };
        let founder = title.contains("ounder");
        let tier = match rng.below(100) {
            0..=5 => Tier::Close,
            6..=25 => Tier::Regular,
            26..=59 => Tier::Occasional,
            60..=69 => Tier::LostTouch,
            _ => Tier::OneOff,
        };
        // Background founders are acquaintances: the warm founders in the
        // demo are the cast's.
        let tier = if founder && matches!(tier, Tier::Close | Tier::Regular) {
            Tier::Occasional
        } else {
            tier
        };
        // Sources: most people you email are also a LinkedIn connection
        // that shares its email, or a contact card; some are LinkedIn-only
        // or card-only.
        let sources = match rng.below(100) {
            0..=24 => src(true, false, false, false),
            25..=44 => src(true, true, true, false),
            45..=59 => src(true, false, false, true),
            60..=69 => src(true, true, true, true),
            70..=91 => src(false, true, false, false),
            _ => src(false, false, false, true),
        };
        let local = match rng.below(3) {
            0 => first.to_lowercase(),
            1 => format!("{}.{}", first.to_lowercase(), last.to_lowercase()),
            _ => format!("{}{}", first[..1].to_lowercase(), last.to_lowercase()),
        };
        // Two people at one company never share an address.
        let mut email = format!("{local}@{}", c.domain);
        if emails.contains(&email) {
            email = format!(
                "{}.{}@{}",
                first.to_lowercase(),
                last.to_lowercase(),
                c.domain
            );
        }
        let phone = (sources.vcf && rng.below(4) != 0).then(|| {
            phones += 1;
            phone(phones)
        });
        let connected = ts(2017, 1, 1, 0, 0) + (rng.unit() * (9.6 * 365.0 * 86_400.0)) as i64;
        names.insert(name);
        emails.insert(email.clone());
        surname_at.insert((ci, last));
        out.push(DemoPerson {
            first: first.into(),
            last: last.into(),
            company: ci,
            title: title.into(),
            email,
            slug: slug(first, last, rng.next_u64()),
            phone,
            sources,
            tier,
            connected,
        });
    }
    out
}

/// Jaro-Winkler similarity (prefix scale 0.1, up to 4 characters), the
/// measure the resolution stream uses for near-name suggestions.
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut b_used = vec![false; b.len()];
    let mut a_match = Vec::new();
    for (i, ca) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        for j in lo..hi {
            if !b_used[j] && b[j] == *ca {
                b_used[j] = true;
                a_match.push(*ca);
                break;
            }
        }
    }
    let m = a_match.len();
    if m == 0 {
        return 0.0;
    }
    let b_match: Vec<char> = b
        .iter()
        .zip(&b_used)
        .filter(|(_, u)| **u)
        .map(|(c, _)| *c)
        .collect();
    let transpositions = a_match.iter().zip(&b_match).filter(|(x, y)| x != y).count() / 2;
    let m = m as f64;
    let jaro = (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions as f64) / m) / 3.0;
    let prefix = a.iter().zip(&b).take(4).take_while(|(x, y)| x == y).count() as f64;
    jaro + prefix * 0.1 * (1.0 - jaro)
}

fn company_index(name: &str) -> usize {
    COMPANIES.iter().position(|c| c.name == name).unwrap()
}

/// Newsletters and notification senders: (display name, address).
pub const NEWSLETTERS: &[(&str, &str)] = &[
    ("Fintech Weekly", "news@fintechweekly.example"),
    ("The Devtools Digest", "digest@devtoolsdigest.example"),
    ("Stackforge Updates", "noreply@stackforge.example"),
    ("Founders Brief", "hello@foundersbrief.example"),
    ("Climate Capital Notes", "notes@climatecapital.example"),
    ("Hexwire Status", "status@hexwire.example"),
    ("Meetup Reminders", "reminders@meetups.example"),
];

// ---------------------------------------------------------------------------
// Mail
// ---------------------------------------------------------------------------

/// One message in the Takeout mbox.
#[derive(Debug, Clone)]
pub struct DemoMail {
    /// Unix seconds.
    pub at: i64,
    /// Gmail thread id.
    pub thread: u64,
    /// Message-ID (without brackets).
    pub id: String,
    /// In-Reply-To (without brackets).
    pub reply_to: Option<String>,
    /// (name, email).
    pub from: (String, String),
    /// (name, email).
    pub to: Vec<(String, String)>,
    /// (name, email).
    pub cc: Vec<(String, String)>,
    /// Subject.
    pub subject: String,
    /// `List-Unsubscribe` present.
    pub bulk: bool,
}

struct MailBox {
    mails: Vec<DemoMail>,
    next_thread: u64,
    /// Addresses that are cc'd but never write.
    silent: BTreeSet<String>,
}

fn owner() -> (String, String) {
    (OWNER_NAME.into(), OWNER_EMAIL.into())
}

fn addr(p: &DemoPerson) -> (String, String) {
    (p.name(), p.email.clone())
}

impl MailBox {
    /// A thread of `n` messages from `start`, `gap` seconds apart (jittered),
    /// between the owner and `people` (first is the main counterpart, the
    /// rest are cc'd). `first_inbound` picks who opens; `replies` says
    /// whether the other side ever answers.
    #[allow(clippy::too_many_arguments)]
    fn thread(
        &mut self,
        rng: &mut Rng,
        subject: &str,
        people: &[(String, String)],
        start: i64,
        gap: i64,
        n: usize,
        first_inbound: bool,
        replies: bool,
    ) {
        self.next_thread += 1;
        let thread = 1_850_000_000_000_000_000 + self.next_thread * 977;
        let mut prev: Option<String> = None;
        let mut at = start;
        let now = demo_now();
        for k in 0..n {
            if at >= now {
                break;
            }
            let inbound = if replies {
                (k % 2 == 0) == first_inbound
            } else {
                first_inbound
            };
            // Inbound messages rotate among the participants (reply-all).
            // People in `silent` are only ever cc'd.
            let sender = if inbound {
                let pick = &people[if k < 2 { 0 } else { k % people.len() }];
                if self.silent.contains(&pick.1) {
                    people[0].clone()
                } else {
                    pick.clone()
                }
            } else {
                owner()
            };
            let mut all: Vec<(String, String)> = std::iter::once(owner())
                .chain(people.iter().cloned())
                .filter(|a| a.1 != sender.1)
                .collect();
            let to = vec![all.remove(0)];
            let id = format!(
                "demo.{}.{}@{}",
                self.next_thread,
                k,
                sender.1.rsplit('@').next().unwrap_or("example.com")
            );
            let subject = if k == 0 {
                subject.to_string()
            } else {
                format!("Re: {subject}")
            };
            self.mails.push(DemoMail {
                at,
                thread,
                id: id.clone(),
                reply_to: prev.take(),
                from: sender,
                to,
                cc: all,
                subject,
                bulk: false,
            });
            prev = Some(id);
            at += gap / 2 + rng.below(gap.max(2) as u64) as i64;
        }
    }
}

fn day(y: i64, m: i64, d: i64) -> i64 {
    ts(y, m, d, 9, 0)
}

const HOUR: i64 = 3600;
const DAYS: i64 = 86_400;

/// The hand-written threads behind the demo questions.
fn scripted_threads(b: &mut MailBox, rng: &mut Rng, by: &BTreeMap<String, DemoPerson>) {
    let p = |n: &str| addr(&by[n]);
    let maya = p("Maya Chen");
    let dana = p("Dana Whitfield");
    let theo = p("Theo Marsh");
    let rafael = p("Rafael Ortiz");
    // Dana is cc'd on other people's threads and never writes to you.
    b.silent.insert(dana.1.clone());

    // Maya Chen: four years of frequent, two-way mail.
    let maya_subjects = [
        "Ledgerline seed round",
        "Coffee Thursday?",
        "Card issuing partners",
        "Intro: Ledgerline <> Harbor Lane",
        "Fintech founders breakfast notes",
        "Payments compliance checklist",
        "Board deck feedback",
        "Hiring a head of risk",
        "Ledgerline Series A timeline",
        "Dinner in New York",
        "Treasury API pricing",
        "Quick favour: reference call",
        "Ledgerline launch week",
        "Fintech regulation roundtable",
    ];
    let mut t = day(2022, 9, 6);
    for (i, s) in maya_subjects.iter().enumerate() {
        let n = 2 + (i % 4);
        b.thread(
            rng,
            s,
            std::slice::from_ref(&maya),
            t,
            6 * HOUR,
            n,
            i % 3 == 0,
            true,
        );
        t += 95 * DAYS + rng.below(20) as i64 * DAYS;
    }
    b.thread(
        rng,
        "Ledgerline Q3 investor update",
        std::slice::from_ref(&maya),
        day(2026, 9, 30),
        5 * HOUR,
        4,
        true,
        true,
    );
    b.thread(
        rng,
        "Fintech founders breakfast, November",
        std::slice::from_ref(&maya),
        day(2026, 10, 9),
        3 * HOUR,
        3,
        false,
        true,
    );

    // The Lumen Pay integration: Maya, Dana and Theo on one thread.
    b.thread(
        rng,
        "Ledgerline x Lumen Pay integration",
        &[maya.clone(), dana.clone(), theo.clone()],
        day(2026, 3, 10),
        20 * HOUR,
        6,
        false,
        true,
    );
    b.thread(
        rng,
        "Lumen Pay sandbox access for Ledgerline",
        &[maya.clone(), dana.clone()],
        day(2026, 7, 2),
        30 * HOUR,
        4,
        true,
        true,
    );

    // Theo Marsh: a warm partnerships contact, often with Dana cc'd.
    for (i, s) in [
        "Lumen Pay partner programme",
        "Partnership terms draft",
        "Lunch next week?",
        "Lumen Pay platform review",
        "Co-marketing for the launch",
        "Lumen Pay roadmap preview",
        "Referral fee question",
        "Conference panel in SF",
    ]
    .iter()
    .enumerate()
    {
        let start = day(2024, 2, 5) + i as i64 * 80 * DAYS;
        let cc_dana = i == 3 || i == 5;
        let ppl = if cc_dana {
            vec![theo.clone(), dana.clone()]
        } else {
            vec![theo.clone()]
        };
        b.thread(rng, s, &ppl, start, 10 * HOUR, 3 + i % 3, i % 2 == 0, true);
    }
    b.thread(
        rng,
        "Lumen Pay API changes for Q4",
        &[theo.clone(), dana.clone()],
        day(2026, 9, 18),
        8 * HOUR,
        4,
        true,
        true,
    );

    // Rafael Ortiz: an investor who sits on Lumen Pay's board.
    for (i, s) in [
        "Meridian portfolio dinner",
        "Intro to two fintech founders",
        "Market map feedback",
        "Office hours slot",
        "Meridian annual meeting",
    ]
    .iter()
    .enumerate()
    {
        let start = day(2023, 4, 3) + i as i64 * 140 * DAYS;
        b.thread(
            rng,
            s,
            std::slice::from_ref(&rafael),
            start,
            12 * HOUR,
            3,
            i % 2 == 0,
            true,
        );
    }
    b.thread(
        rng,
        "Lumen Pay board prep",
        &[rafael.clone(), dana.clone()],
        day(2026, 6, 12),
        18 * HOUR,
        4,
        false,
        true,
    );
    b.thread(
        rng,
        "Lumen Pay technical diligence",
        &[rafael.clone(), dana.clone()],
        day(2025, 11, 4),
        20 * HOUR,
        3,
        true,
        true,
    );

    // Other Lumen Pay people.
    b.thread(
        rng,
        "Webhook retries on Lumen Pay",
        &[p("Ines Duarte")],
        day(2025, 8, 19),
        6 * HOUR,
        4,
        true,
        true,
    );
    b.thread(
        rng,
        "Lumen Pay status page",
        &[p("Ines Duarte")],
        day(2026, 2, 2),
        6 * HOUR,
        2,
        true,
        true,
    );
    b.thread(
        rng,
        "Lumen Pay dashboard feedback",
        &[p("Kwame Asante")],
        day(2026, 5, 27),
        9 * HOUR,
        3,
        false,
        true,
    );
    b.thread(
        rng,
        "Hiring at Meridian portfolio companies",
        &[p("Aisha Bello")],
        day(2025, 10, 1),
        30 * HOUR,
        3,
        true,
        true,
    );
    b.thread(
        rng,
        "Talent intros for Q1",
        &[p("Aisha Bello")],
        day(2026, 1, 13),
        30 * HOUR,
        2,
        true,
        true,
    );

    // The other fintech founders, and the breakfast that ties them together.
    let founders = [
        (
            "Arjun Mehta",
            [
                "Tallyfin pricing",
                "Fintech regulation in the UK",
                "Tallyfin seed close",
                "Catch up in London",
            ],
        ),
        (
            "Sofia Brandt",
            [
                "Fernpay open banking demo",
                "Fintech meetup Berlin",
                "Fernpay engineering hiring",
                "Ledger reconciliation",
            ],
        ),
        (
            "Lucas Moreau",
            [
                "Brightvault intro",
                "Fintech licensing in Singapore",
                "Brightvault pilot",
                "Coffee when you're in town",
            ],
        ),
        (
            "Nadia Petrova",
            [
                "Quaystone lending pilot",
                "Fintech ops benchmarks",
                "Quaystone board seat",
                "Toronto fintech dinner",
            ],
        ),
    ];
    for (fi, (name, subjects)) in founders.iter().enumerate() {
        let who = p(name);
        for (i, s) in subjects.iter().enumerate() {
            let start = day(2024, 3, 1) + (fi as i64 * 23 + i as i64 * 150) * DAYS;
            b.thread(
                rng,
                s,
                std::slice::from_ref(&who),
                start,
                9 * HOUR,
                2 + (i + fi) % 3,
                i % 2 == 1,
                true,
            );
        }
    }
    b.thread(
        rng,
        "Fintech founders breakfast",
        &[
            p("Arjun Mehta"),
            p("Sofia Brandt"),
            p("Lucas Moreau"),
            p("Nadia Petrova"),
        ],
        day(2026, 4, 14),
        5 * HOUR,
        5,
        false,
        true,
    );

    // Non-fintech founders: warm, but on other topics.
    for (name, subjects) in [
        ("Hana Sato", ["Coinwell token launch", "Lisbon crypto week"]),
        (
            "Owen Gallagher",
            ["Stackforge CLI beta", "Devtools dinner in SF"],
        ),
        (
            "Leila Haddad",
            ["Greenloop carbon pilot", "Climate founders dinner"],
        ),
        (
            "Tomas Rivera",
            ["Juniper Health clinic rollout", "Healthtech data partners"],
        ),
    ] {
        let who = p(name);
        for (i, s) in subjects.iter().enumerate() {
            let start = day(2025, 1, 15) + i as i64 * 170 * DAYS + rng.below(40) as i64 * DAYS;
            b.thread(
                rng,
                s,
                std::slice::from_ref(&who),
                start,
                8 * HOUR,
                3,
                i == 0,
                true,
            );
        }
    }

    // Developer tools people.
    for (name, subjects) in [
        (
            "Elif Kaya",
            ["Stackforge SDK workshop", "Developer relations meetup"],
        ),
        (
            "Priya Natarajan",
            ["Hexwire API rate limits", "API platform migration"],
        ),
        (
            "Ben Adler",
            ["Quillstack SDK release notes", "Client library naming"],
        ),
        (
            "Grace Liu",
            ["Driftless developer conference", "DevRel playbook"],
        ),
    ] {
        let who = p(name);
        for (i, s) in subjects.iter().enumerate() {
            let start = day(2025, 4, 2) + i as i64 * 120 * DAYS + rng.below(30) as i64 * DAYS;
            b.thread(
                rng,
                s,
                std::slice::from_ref(&who),
                start,
                7 * HOUR,
                2 + i,
                i == 1,
                true,
            );
        }
    }

    // Noah Lindqvist: work mail only (his LinkedIn row has no email).
    b.thread(
        rng,
        "Tallyfin growth experiments",
        &[p("Noah Lindqvist")],
        day(2025, 9, 9),
        10 * HOUR,
        3,
        true,
        true,
    );

    // Marco Bianchi: his work and personal addresses on one thread.
    let marco = p("Marco Bianchi");
    let marco_home = (marco.0.clone(), "marco@bianchi-home.example".to_string());
    b.thread(
        rng,
        "Kitebox brand refresh",
        &[marco.clone(), marco_home],
        day(2025, 12, 1),
        20 * HOUR,
        3,
        true,
        true,
    );
    b.thread(
        rng,
        "Kitebox design review",
        std::slice::from_ref(&marco),
        day(2026, 3, 3),
        20 * HOUR,
        2,
        true,
        true,
    );

    // Hollis Grant: close in 2021-2023, then silence.
    let hollis = p("Hollis Grant");
    for (i, s) in [
        "Ridgeway platform rewrite",
        "1:1 notes",
        "Offsite agenda",
        "On-call rotation",
        "Engineering ladder draft",
        "Ridgeway hiring plan",
        "Farewell drinks",
    ]
    .iter()
    .enumerate()
    {
        let start = day(2021, 3, 1) + i as i64 * 95 * DAYS;
        b.thread(
            rng,
            s,
            std::slice::from_ref(&hollis),
            start,
            5 * HOUR,
            4 + i % 3,
            i % 2 == 0,
            true,
        );
    }
}

const TOPICS_GENERIC: &[&str] = &[
    "Quick intro",
    "Coffee next week?",
    "Following up",
    "Notes from today",
    "Catching up",
    "Reference check",
    "Hiring for a senior engineer",
    "Conference speaker invite",
    "Advice on pricing",
    "Customer interview request",
    "Partnership idea",
    "Thanks for the intro",
    "Q3 planning",
    "Podcast invitation",
];

fn industry_topics(industry: &str) -> &'static [&'static str] {
    match industry {
        "fintech" => &[
            "Payments partnership",
            "Card program launch",
            "Fraud tooling",
            "Treasury integration",
        ],
        "crypto" => &["Wallet integration", "Stablecoin settlement"],
        "devtools" => &[
            "SDK feedback",
            "API design review",
            "Developer onboarding",
            "CLI release",
        ],
        "healthtech" => &["Clinical data pilot", "Patient intake flow"],
        "climate" => &["Carbon accounting pilot", "Grid storage project"],
        "robotics" => &["Warehouse robot trial", "Robot fleet telemetry"],
        "hardware" => &["Prototype run", "Supply chain update"],
        "consumer" => &["App launch feedback", "Retention experiments"],
        "saas" => &[
            "Enterprise rollout",
            "Renewal discussion",
            "SOC 2 questions",
        ],
        "security" => &["Pen test results", "Security review"],
        "edtech" => &["Course platform pilot", "Learning analytics"],
        "gaming" => &["Playtest invite", "Game studio partnership"],
        "vc" => &["Portfolio intro", "Fund update", "Office hours"],
        "legal" => &["Term sheet review", "Option pool paperwork"],
        "design" => &["Brand workshop", "Design system audit"],
        "media" => &["Interview request", "Sponsored episode"],
        "logistics" => &["Freight pilot", "Route optimisation"],
        "biotech" => &["Lab automation", "Assay data pipeline"],
        "ai" => &["Model evaluation", "Inference costs"],
        _ => &[],
    }
}

fn background_threads(b: &mut MailBox, rng: &mut Rng, people: &[DemoPerson]) {
    let now = demo_now();
    let by_company: BTreeMap<usize, Vec<usize>> = people
        .iter()
        .enumerate()
        .filter(|(_, p)| p.sources.mbox && p.tier != Tier::Scripted)
        .fold(BTreeMap::new(), |mut m, (i, p)| {
            m.entry(p.company).or_insert_with(Vec::new).push(i);
            m
        });
    for (i, p) in people.iter().enumerate() {
        if !p.sources.mbox || p.tier == Tier::Scripted {
            continue;
        }
        let c = p.company();
        let (threads, span_end, span_len, reply) = match p.tier {
            Tier::Close => (
                14 + rng.below(12),
                now - rng.below(20) as i64 * DAYS,
                900 * DAYS,
                95,
            ),
            Tier::Regular => (
                5 + rng.below(7),
                now - (30 + rng.below(200)) as i64 * DAYS,
                700 * DAYS,
                80,
            ),
            Tier::Occasional => (
                2 + rng.below(3),
                now - rng.below(900) as i64 * DAYS,
                500 * DAYS,
                55,
            ),
            Tier::LostTouch => (
                7 + rng.below(8),
                now - (800 + rng.below(600)) as i64 * DAYS,
                600 * DAYS,
                90,
            ),
            Tier::OneOff | Tier::Scripted => (1, now - rng.below(1200) as i64 * DAYS, DAYS, 20),
        };
        for k in 0..threads {
            let frac = if threads == 1 {
                1.0
            } else {
                k as f64 / (threads - 1) as f64
            };
            let start =
                span_end - span_len + (frac * span_len as f64) as i64 - rng.below(5) as i64 * DAYS;
            let topics = industry_topics(c.industry);
            let subject = match rng.below(4) {
                0 | 1 if !topics.is_empty() => {
                    format!(
                        "{}: {}",
                        c.name,
                        topics[rng.below(topics.len() as u64) as usize]
                    )
                }
                2 if !topics.is_empty() => {
                    topics[rng.below(topics.len() as u64) as usize].to_string()
                }
                _ => TOPICS_GENERIC[rng.below(TOPICS_GENERIC.len() as u64) as usize].to_string(),
            };
            let mut ppl = vec![addr(p)];
            // A colleague cc'd now and then: ties inside companies.
            if p.tier != Tier::OneOff && rng.below(4) == 0 {
                if let Some(cs) = by_company.get(&p.company) {
                    let j = cs[rng.below(cs.len() as u64) as usize];
                    if j != i {
                        ppl.push(addr(&people[j]));
                    }
                }
            }
            let replies = rng.below(100) < reply;
            let n = match p.tier {
                Tier::OneOff => 1,
                Tier::Close => 2 + rng.below(5) as usize,
                _ => 1 + rng.below(4) as usize,
            };
            let inbound = p.tier == Tier::OneOff || rng.below(2) == 0;
            let gap = (4 + rng.below(40) as i64) * HOUR;
            b.thread(rng, &subject, &ppl, start, gap, n, inbound, replies);
        }
    }
}

fn newsletters(b: &mut MailBox, rng: &mut Rng) {
    for (ni, (name, email)) in NEWSLETTERS.iter().enumerate() {
        let mut at = ts(2024, 1, 3, 7, 0) + ni as i64 * 13 * HOUR;
        let mut issue = 1;
        while at < demo_now() {
            b.next_thread += 1;
            let thread = 1_850_000_000_000_000_000 + b.next_thread * 977;
            b.mails.push(DemoMail {
                at,
                thread,
                id: format!("demo.n{ni}.{issue}@{}", email.rsplit('@').next().unwrap()),
                reply_to: None,
                from: ((*name).into(), (*email).into()),
                to: vec![owner()],
                cc: vec![],
                subject: format!("{name} #{issue}"),
                bulk: true,
            });
            issue += 1;
            at += (10 + rng.below(9) as i64) * DAYS;
        }
    }
}

/// Every message, in time order.
pub fn mails(people: &[DemoPerson]) -> Vec<DemoMail> {
    let mut rng = Rng::new(0x0005_EED5);
    let by: BTreeMap<String, DemoPerson> = people.iter().map(|p| (p.name(), p.clone())).collect();
    let mut b = MailBox {
        mails: Vec::new(),
        next_thread: 0,
        silent: BTreeSet::new(),
    };
    scripted_threads(&mut b, &mut rng, &by);
    background_threads(&mut b, &mut rng, people);
    newsletters(&mut b, &mut rng);
    b.mails.retain(|m| m.at < demo_now());
    b.mails
        .sort_by(|x, y| x.at.cmp(&y.at).then_with(|| x.id.cmp(&y.id)));
    b.mails
}

fn addr_list(list: &[(String, String)]) -> String {
    list.iter()
        .map(|(n, e)| {
            if n.contains([',', '&', '\'']) {
                format!("\"{n}\" <{e}>")
            } else {
                format!("{n} <{e}>")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The Takeout-style mbox (headers plus a one-line placeholder body).
pub fn mbox_text(mails: &[DemoMail]) -> String {
    let mut out = String::with_capacity(mails.len() * 420);
    for m in mails {
        let sent = m.from.1 == OWNER_EMAIL;
        let _ = writeln!(out, "From {} {}", m.from.1, from_line_date(m.at));
        let _ = writeln!(out, "X-GM-THRID: {}", m.thread);
        let labels = if sent {
            "Sent"
        } else if m.bulk {
            "Inbox,Category Updates"
        } else {
            "Inbox"
        };
        let _ = writeln!(out, "X-Gmail-Labels: {labels}");
        let _ = writeln!(out, "Message-ID: <{}>", m.id);
        let _ = writeln!(out, "Date: {}", rfc2822(m.at));
        let _ = writeln!(out, "From: {}", addr_list(std::slice::from_ref(&m.from)));
        let _ = writeln!(out, "To: {}", addr_list(&m.to));
        if !m.cc.is_empty() {
            let _ = writeln!(out, "Cc: {}", addr_list(&m.cc));
        }
        let _ = writeln!(out, "Subject: {}", m.subject);
        if let Some(r) = &m.reply_to {
            let _ = writeln!(out, "In-Reply-To: <{r}>");
        }
        if m.bulk {
            let domain = m.from.1.rsplit('@').next().unwrap_or("example.com");
            let _ = writeln!(out, "List-Unsubscribe: <https://{domain}/unsubscribe>");
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "(synthetic demo message; bodies are never read)");
        let _ = writeln!(out);
    }
    out
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// LinkedIn `Connections.csv` (BOM, `Notes:` preamble, CRLF), oldest first.
pub fn connections_csv(people: &[DemoPerson]) -> String {
    let mut rows: Vec<&DemoPerson> = people.iter().filter(|p| p.sources.linkedin).collect();
    rows.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.slug.cmp(&b.slug))
    });
    let mut out = String::from("\u{feff}Notes:\r\n");
    out.push_str("\"When exporting your connection data, you may notice that some of the email addresses are missing. You will only see email addresses for connections who have allowed their connections to see or download their email address.\"\r\n");
    out.push_str("\r\n");
    out.push_str("First Name,Last Name,URL,Email Address,Company,Position,Connected On\r\n");
    for p in rows {
        let email = if p.sources.linkedin_email {
            p.email.as_str()
        } else {
            ""
        };
        let fields = [
            p.first.as_str(),
            p.last.as_str(),
            &format!("https://www.linkedin.com/in/{}", p.slug),
            email,
            p.company().name,
            p.title.as_str(),
            &linkedin_date(p.connected),
        ];
        let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

fn vcard_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

/// `contacts.vcf` (vCard 3.0, as exported by Apple or Google Contacts).
pub fn vcf_text(people: &[DemoPerson]) -> String {
    let mut out = String::new();
    for (i, p) in people.iter().enumerate().filter(|(_, p)| p.sources.vcf) {
        let c = p.company();
        let mut lines = vec![
            "BEGIN:VCARD".to_string(),
            "VERSION:3.0".to_string(),
            "PRODID:-//openstance demo//contacts//EN".to_string(),
            format!("N:{};{};;;", vcard_escape(&p.last), vcard_escape(&p.first)),
            format!("FN:{}", vcard_escape(&p.name())),
            format!("ORG:{}", vcard_escape(c.name)),
            format!("TITLE:{}", vcard_escape(&p.title)),
        ];
        // Card-only people without a phone still carry their work email.
        if p.sources.mbox || p.phone.is_none() || i % 2 == 0 {
            lines.push(format!("EMAIL;type=INTERNET;type=WORK:{}", p.email));
        }
        if let Some(ph) = &p.phone {
            // +14155550142 -> +1 (415) 555-0142
            let d = &ph[2..];
            lines.push(format!(
                "TEL;type=CELL:+1 ({}) {}-{}",
                &d[..3],
                &d[3..6],
                &d[6..]
            ));
        }
        lines.push(format!(
            "ADR;type=WORK:;;;{};{};;{}",
            c.city, c.region, c.country
        ));
        lines.push(format!(
            "REV:{}",
            vcard_rev(ts(2026, 1, 1, 0, 0) + i as i64 * 3600)
        ));
        lines.push("END:VCARD".to_string());
        for l in lines {
            out.push_str(&l);
            out.push_str("\r\n");
        }
    }
    out
}

/// Every demo file: name under `fixtures/demo/` -> contents.
pub fn files() -> BTreeMap<&'static str, String> {
    let people = people();
    let mut f = BTreeMap::new();
    f.insert("takeout.mbox", mbox_text(&mails(&people)));
    f.insert("Connections.csv", connections_csv(&people));
    f.insert("contacts.vcf", vcf_text(&people));
    f
}

/// Write the demo files into `dir` (the repository's `fixtures/demo/`).
pub fn generate(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (rel, text) in files() {
        std::fs::write(dir.join(rel), text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_six_hundred_people_with_unique_names() {
        let ppl = people();
        assert!((590..=620).contains(&ppl.len()), "{}", ppl.len());
        let names: BTreeSet<String> = ppl.iter().map(DemoPerson::name).collect();
        assert_eq!(names.len(), ppl.len(), "duplicate names");
        let emails: BTreeSet<&str> = ppl.iter().map(|p| p.email.as_str()).collect();
        assert_eq!(emails.len(), ppl.len(), "duplicate emails");
        let phones: Vec<&String> = ppl.iter().filter_map(|p| p.phone.as_ref()).collect();
        let distinct: BTreeSet<&&String> = phones.iter().collect();
        assert_eq!(distinct.len(), phones.len(), "duplicate phones");
    }

    #[test]
    fn only_reserved_domains_and_fictional_phones() {
        let f = files();
        for p in people() {
            assert!(p.email.ends_with(".example"), "{}", p.email);
            if let Some(ph) = &p.phone {
                assert!(ph.len() == 12 && &ph[5..10] == "55501", "{ph}");
            }
        }
        for m in mails(&people()) {
            for (_, e) in std::iter::once(&m.from).chain(&m.to).chain(&m.cc) {
                assert!(e.ends_with(".example") || e == OWNER_EMAIL, "{e}");
            }
        }
        assert!(f["takeout.mbox"].len() < 4 * 1024 * 1024, "mbox too large");
    }

    #[test]
    fn jaro_winkler_matches_the_reference_values() {
        // Values from the Phase 1 review queue (strsim).
        assert!((jaro_winkler("aria yamamoto", "daria yamamoto") - 0.9762).abs() < 1e-3);
        assert!((jaro_winkler("jon quillby", "john quillby") - 0.9778).abs() < 1e-3);
        assert!((jaro_winkler("malik hartigan", "malik varga") - 0.9247).abs() < 1e-3);
        assert_eq!(jaro_winkler("abc", "abc"), 1.0);
    }

    #[test]
    fn no_two_names_are_near_identical() {
        let names: Vec<String> = people().iter().map(|p| p.name().to_lowercase()).collect();
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                assert!(jaro_winkler(a, b) < 0.92, "{a} ~ {b}");
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(files(), files());
    }

    #[test]
    fn every_cast_member_exists_once() {
        let ppl = people();
        for n in FLAGGED
            .iter()
            .chain(FINTECH_FOUNDERS)
            .chain(LUMEN_PAY)
            .chain(DANA_MUTUALS)
            .chain(DEVTOOLS)
        {
            assert_eq!(ppl.iter().filter(|p| &p.name() == n).count(), 1, "{n}");
        }
        assert_eq!(company("Lumen Pay").domain, "lumenpay.example");
    }
}
