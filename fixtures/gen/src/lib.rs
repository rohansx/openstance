//! Deterministic generator for the synthetic Phase 1 fixtures (P1-CI-03).
//!
//! Everything here is fictional: names are invented, domains are reserved
//! (`*.example`, `*.test`, `*.invalid`, `example.com`) and phone numbers use
//! the fictional `+1 NXX 555-01xx` range. The cast and its expectations live
//! in [`CAST`]; the files are written by [`generate`] and the contract table
//! by [`expected_md`]. Run `cargo run -p openstance-fixtures --bin
//! gen-fixtures` to regenerate `fixtures/`; a test fails if the committed
//! files drift from the generator.

#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

pub mod graph;

/// Planted in every body / note / DM content: must never reach the database.
pub const BODY_SENTINEL: &str = "SENTINEL-BODY-7f3a";

/// The owner of the fixture graph.
pub const OWNER_NAME: &str = "Owen Ryder";
/// Owner email.
pub const OWNER_EMAIL: &str = "owner@example.com";
/// Owner LinkedIn slug.
pub const OWNER_SLUG: &str = "owen-ryder-example";

/// "Today" for anything computed over the fixtures (2026-10-01T00:00:00Z).
pub fn fixture_now() -> i64 {
    ts(2026, 10, 1, 0, 0)
}

// ---------------------------------------------------------------------------
// Calendar helpers (no chrono dependency)
// ---------------------------------------------------------------------------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Unix seconds for a UTC date and time.
pub fn ts(y: i64, m: i64, d: i64, h: i64, mi: i64) -> i64 {
    days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60
}

const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

struct Parts {
    y: i64,
    mo: i64,
    d: i64,
    h: i64,
    mi: i64,
    s: i64,
    wd: &'static str,
}

fn parts(t: i64) -> Parts {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    Parts {
        y,
        mo,
        d,
        h: secs / 3600,
        mi: secs % 3600 / 60,
        s: secs % 60,
        wd: WEEKDAYS[days.rem_euclid(7) as usize],
    }
}

/// `Mon, 06 Jan 2025 09:00:00 +0000`.
pub fn rfc2822(t: i64) -> String {
    let p = parts(t);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        p.wd,
        p.d,
        MONTHS[(p.mo - 1) as usize],
        p.y,
        p.h,
        p.mi,
        p.s
    )
}

/// mbox `From ` line date: `Mon Jan  6 09:00:00 2025`.
pub fn from_line_date(t: i64) -> String {
    let p = parts(t);
    format!(
        "{} {} {:>2} {:02}:{:02}:{:02} {}",
        p.wd,
        MONTHS[(p.mo - 1) as usize],
        p.d,
        p.h,
        p.mi,
        p.s,
        p.y
    )
}

/// LinkedIn `Connected On`: `12 Mar 2024`.
pub fn linkedin_date(t: i64) -> String {
    let p = parts(t);
    format!("{:02} {} {}", p.d, MONTHS[(p.mo - 1) as usize], p.y)
}

/// LinkedIn messages `DATE`: `2025-03-04 10:00:00 UTC`.
pub fn linkedin_datetime(t: i64) -> String {
    let p = parts(t);
    format!(
        "{}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        p.y, p.mo, p.d, p.h, p.mi, p.s
    )
}

/// LinkedIn Invitations `Sent At`: `9/15/25, 10:00 AM`.
pub fn linkedin_sent_at(t: i64) -> String {
    let p = parts(t);
    let (h12, ampm) = match p.h {
        0 => (12, "AM"),
        1..=11 => (p.h, "AM"),
        12 => (12, "PM"),
        _ => (p.h - 12, "PM"),
    };
    format!(
        "{}/{}/{:02}, {}:{:02} {}",
        p.mo,
        p.d,
        p.y % 100,
        h12,
        p.mi,
        ampm
    )
}

/// vCard `REV`: `20250301T120000Z`.
pub fn vcard_rev(t: i64) -> String {
    let p = parts(t);
    format!(
        "{}{:02}{:02}T{:02}{:02}{:02}Z",
        p.y, p.mo, p.d, p.h, p.mi, p.s
    )
}

// ---------------------------------------------------------------------------
// The cast and its expectations
// ---------------------------------------------------------------------------

/// A resolution expectation for one cast member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    /// One identity, nothing to merge.
    Single,
    /// Merged by rule 1 (same handle or co-reference), confidence 1.0.
    Exact,
    /// Merged by rule 2 (gmail dots / plus aliasing), 0.98.
    Alias,
    /// Merged and flagged by rule 3 (name + company), 0.85.
    FlaggedNameCompany,
    /// Merged and flagged by rule 4 (name + mutual thread), 0.75.
    FlaggedNameThread,
}

/// One fictional person (a cast entry may be one of two people sharing a name).
#[derive(Debug, Clone, Copy)]
pub struct CastMember {
    /// Stable key; also used as the expected person key.
    pub key: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Expected merge outcome.
    pub merge: Merge,
    /// Expected `is_bulk = 1`.
    pub bulk: bool,
    /// Scenario description (EXPECTED.md).
    pub scenario: &'static str,
}

/// A review suggestion the fixtures must produce (never auto-merged).
#[derive(Debug, Clone, Copy)]
pub struct ExpectedSuggestion {
    /// One side.
    pub a: &'static str,
    /// Other side.
    pub b: &'static str,
    /// `rare_name` or `jaro_winkler`.
    pub rule: &'static str,
    /// Why.
    pub note: &'static str,
}

const fn m(
    key: &'static str,
    name: &'static str,
    merge: Merge,
    bulk: bool,
    scenario: &'static str,
) -> CastMember {
    CastMember {
        key,
        name,
        merge,
        bulk,
        scenario,
    }
}

/// The cast. `self` is the owner (the self person, `is_self = 1`).
pub const CAST: &[CastMember] = &[
    m("self", OWNER_NAME, Merge::Exact, false, "the owner: [me] email + LinkedIn slug must-linked into the self person"),
    m("priya", "Priya Raman", Merge::Exact, false, "same email in mbox, LinkedIn (Email Address) and vcf; busy two-way correspondent with 52 inbound messages that must NOT be bulk"),
    m("marcus", "Marcus Lee", Merge::Exact, false, "mbox + LinkedIn row carrying the same email; cc'd with Priya on exactly 3 messages (tie n_shared = 3)"),
    m("sam", "Sam Ito", Merge::FlaggedNameCompany, false, "LinkedIn row without email (Company Northwind) + mbox sam.ito@northwind.example"),
    m("lena", "Lena Park", Merge::FlaggedNameCompany, false, "vcf (ORG Contoso, freemail address) + LinkedIn row without email (Company Contoso)"),
    m("tariq", "Tariq Bellweather", Merge::FlaggedNameThread, false, "work and personal addresses with the same display name on one email thread"),
    m("dana", "Dana Whitfield", Merge::Alias, false, "gmail.test dot and plus aliases plus googlemail.test (requires fixtures/config.toml)"),
    m("pat", "Pat Moreno", Merge::Alias, false, "corporate plus-addressing pat+hiring@ / pat@fabrikam.example"),
    m("alex_nw", "Alex Kim", Merge::Single, false, "common-name collision, side 1: LinkedIn at Northwind; must stay separate, no suggestion"),
    m("alex_ct", "Alex Kim", Merge::Single, false, "common-name collision, side 2: mbox at contoso.example"),
    m("ines_li", "Ines Quillfeather", Merge::Single, false, "rare identical name, side 1: LinkedIn at Halcyon; not merged, one rule-5 suggestion"),
    m("ines_vcf", "Ines Quillfeather", Merge::Single, false, "rare identical name, side 2: vcf with a freemail address only"),
    m("jon", "Jon Quillby", Merge::Single, false, "Jaro-Winkler near-miss, side 1: vcf at Brightwater"),
    m("john", "John Quillby", Merge::Single, false, "Jaro-Winkler near-miss, side 2: LinkedIn at Brightwater; suggestion only"),
    m("digest", "Digest Weekly", Merge::Single, true, "newsletter: 60 messages, List-Unsubscribe on all, zero replies -> bulk"),
    m("rufus", "Rufus Pennyworth", Merge::Single, false, "phone-only vCard -> person with a phone identity"),
    m("mira", "Mira Castellanos", Merge::Exact, false, "one vCard (4.0) with two emails + phone; one email also in mbox"),
    m("kofi", "Kofi Mensah", Merge::Single, false, "same email in mbox (RFC 2047 display name with emoji) and vcf"),
    m("greta", "Greta Solberg", Merge::Single, false, "pending outgoing LinkedIn invitation only"),
    m("hugo", "Hugo Brandt", Merge::Single, false, "LinkedIn DM only (not a connection)"),
    m("elena", "Elena Voss", Merge::Single, false, "mbox only, display name written \"Voss, Elena\""),
    m("ravi", "Ravi Iyer", Merge::Single, false, "mbox only, two-way; one message duplicated verbatim (same Message-ID)"),
    m("nora", "Nora Lindqvist", Merge::Single, false, "mbox only, one cold inbound message"),
    m("chidi", "Chidi Okafor", Merge::Single, false, "LinkedIn connection only"),
    m("sofia", "Sofia Marchetti", Merge::Single, false, "LinkedIn connection only (quoted comma in Position)"),
    m("kenji", "Kenji Watanabe", Merge::Single, false, "LinkedIn connection only (Northwind)"),
    m("amara", "Amara Nwosu", Merge::Single, false, "LinkedIn connection + DMs"),
    m("beatriz", "Beatriz Okonkwo", Merge::Single, false, "vcf only (freemail)"),
    m("lars", "Lars Eriksen", Merge::Single, false, "vcf only (grouped item1.EMAIL property)"),
    m("jose", "José García", Merge::FlaggedNameCompany, false, "vCard 2.1 quoted-printable \"José García\" at tailspin.example + LinkedIn \"Jose Garcia\" at Tailspin (diacritics folded)"),
    m("hannah", "Hannah O'Neil", Merge::Single, false, "same email in vcf (\"Dr. Hannah O'Neil\") and mbox (RFC 2047 encoded name)"),
];

/// Suggestions `openstance review` must list for the fixtures.
pub const SUGGESTIONS: &[ExpectedSuggestion] = &[
    ExpectedSuggestion {
        a: "ines_li",
        b: "ines_vcf",
        rule: "rare_name",
        note: "identical uncommon name, nothing shared (confidence 0.5)",
    },
    ExpectedSuggestion {
        a: "jon",
        b: "john",
        rule: "jaro_winkler",
        note: "jaro_winkler(\"jon quillby\", \"john quillby\") = 0.9778 >= 0.92, both names rare, shared company (confidence 0.5 x 0.9778 = 0.4889)",
    },
];

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// What the importer must do with a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailFate {
    /// Imported as an email interaction.
    Import,
    /// Skipped: unparseable `Date` and `From ` line (`bad_date`).
    SkipBadDate,
    /// Skipped: labelled Spam (`unsupported`).
    SkipSpam,
    /// Same Message-ID as an earlier message in the file (`interactions_dup`).
    Duplicate,
}

/// An address on a message: (display name, email, cast key).
type Addr = (&'static str, String, &'static str);

/// One mbox message.
#[derive(Debug, Clone)]
pub struct Mail {
    /// Message-ID without brackets.
    pub id: String,
    /// X-GM-THRID, if any.
    pub thrid: Option<u64>,
    /// In-Reply-To / References target, if any.
    pub reply_to: Option<String>,
    /// Unix seconds.
    pub at: i64,
    /// Sender.
    pub from: Addr,
    /// To.
    pub to: Vec<Addr>,
    /// Cc.
    pub cc: Vec<Addr>,
    /// Subject (raw header value, may be RFC 2047 encoded).
    pub subject: String,
    /// X-Gmail-Labels.
    pub labels: &'static str,
    /// Has List-Unsubscribe.
    pub list_unsub: bool,
    /// Raw `From:` header value override (RFC 2047 names).
    pub from_header: Option<String>,
    /// Write `To: undisclosed-recipients:;`.
    pub undisclosed: bool,
    /// Expected importer behaviour.
    pub fate: MailFate,
}

fn owner() -> Addr {
    (OWNER_NAME, OWNER_EMAIL.to_string(), "self")
}

fn a(name: &'static str, email: &str, key: &'static str) -> Addr {
    (name, email.to_string(), key)
}

struct MailBuilder {
    mails: Vec<Mail>,
    next_thread: u64,
    seq: u32,
}

impl MailBuilder {
    fn thread(&mut self) -> u64 {
        self.next_thread += 1;
        1_810_000_000_000_000 + self.next_thread
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        domain: &str,
        thrid: Option<u64>,
        reply_to: Option<String>,
        at: i64,
        from: Addr,
        to: Vec<Addr>,
        cc: Vec<Addr>,
        subject: &str,
    ) -> String {
        self.seq += 1;
        let id = format!("m{:04}.{}@{}", self.seq, at, domain);
        let labels = if from.2 == "self" {
            "Sent"
        } else {
            "Inbox,Category Personal"
        };
        self.mails.push(Mail {
            id: id.clone(),
            thrid,
            reply_to,
            at,
            from,
            to,
            cc,
            subject: subject.to_string(),
            labels,
            list_unsub: false,
            from_header: None,
            undisclosed: false,
            fate: MailFate::Import,
        });
        id
    }
}

/// Every mbox message, in file order.
pub fn mails() -> Vec<Mail> {
    let mut b = MailBuilder {
        mails: Vec::new(),
        next_thread: 0,
        seq: 0,
    };
    let me = owner;
    let priya = || a("Priya Raman", "priya@northwind.example", "priya");
    let marcus = || a("Marcus Lee", "marcus@contoso.example", "marcus");

    // Priya: 26 threads she starts; owner replies; she replies again.
    for i in 0..26 {
        let t = b.thread();
        let day = ts(2025, 1, 6, 9, 0) + i * 21 * 86_400;
        let subj = format!("Northwind platform sync #{}", i + 1);
        let m1 = b.push(
            "northwind.example",
            Some(t),
            None,
            day,
            priya(),
            vec![me()],
            vec![],
            &subj,
        );
        let m2 = b.push(
            "example.com",
            Some(t),
            Some(m1),
            day + 3 * 3600,
            me(),
            vec![priya()],
            vec![],
            &format!("Re: {subj}"),
        );
        b.push(
            "northwind.example",
            Some(t),
            Some(m2),
            day + 6 * 3600,
            priya(),
            vec![me()],
            vec![],
            &format!("Re: {subj}"),
        );
    }
    // Priya and Marcus cc'd together on exactly three messages.
    for (i, subj) in [
        "Intro: Priya <> Marcus",
        "Infra hiring loop",
        "Q3 architecture review",
    ]
    .iter()
    .enumerate()
    {
        let t = b.thread();
        let at = ts(2026, 6, 2, 8, 0) + i as i64 * 7 * 86_400;
        b.push(
            "example.com",
            Some(t),
            None,
            at,
            me(),
            vec![priya()],
            vec![marcus()],
            subj,
        );
    }
    // Marcus direct: two short threads.
    for i in 0..2 {
        let t = b.thread();
        let at = ts(2025, 11, 3, 15, 0) + i * 30 * 86_400;
        let m1 = b.push(
            "contoso.example",
            Some(t),
            None,
            at,
            marcus(),
            vec![me()],
            vec![],
            "Coffee next week?",
        );
        b.push(
            "example.com",
            Some(t),
            Some(m1),
            at + 7200,
            me(),
            vec![marcus()],
            vec![],
            "Re: Coffee next week?",
        );
    }
    // Sam Ito: one thread in, out, in.
    {
        let sam = || a("Sam Ito", "sam.ito@northwind.example", "sam");
        let t = b.thread();
        let at = ts(2026, 2, 10, 10, 0);
        let m1 = b.push(
            "northwind.example",
            Some(t),
            None,
            at,
            sam(),
            vec![me()],
            vec![],
            "Roadmap feedback",
        );
        let m2 = b.push(
            "example.com",
            Some(t),
            Some(m1),
            at + 3600,
            me(),
            vec![sam()],
            vec![],
            "Re: Roadmap feedback",
        );
        b.push(
            "northwind.example",
            Some(t),
            Some(m2),
            at + 7200,
            sam(),
            vec![me()],
            vec![],
            "Re: Roadmap feedback",
        );
    }
    // Tariq: work + personal address on one thread (no X-GM-THRID: thread
    // key comes from Message-ID / In-Reply-To), plus two work-only threads.
    {
        let work = || a("Tariq Bellweather", "tariq@fabrikam.example", "tariq");
        let personal = || a("Tariq Bellweather", "tariq.bell@mail.test", "tariq");
        let at = ts(2026, 4, 14, 9, 30);
        let m1 = b.push(
            "example.com",
            None,
            None,
            at,
            me(),
            vec![work()],
            vec![personal()],
            "Weekend climbing plan",
        );
        b.push(
            "mail.test",
            None,
            Some(m1),
            at + 5400,
            personal(),
            vec![me()],
            vec![],
            "Re: Weekend climbing plan",
        );
        for i in 0..2 {
            let t = b.thread();
            let at = ts(2026, 5, 4, 11, 0) + i * 14 * 86_400;
            b.push(
                "fabrikam.example",
                Some(t),
                None,
                at,
                work(),
                vec![me()],
                vec![],
                "Fabrikam partnership notes",
            );
        }
    }
    // Dana: gmail.test dot/plus aliases and googlemail.test, separate threads.
    {
        let t = b.thread();
        b.push(
            "gmail.test",
            Some(t),
            None,
            ts(2025, 9, 1, 18, 0),
            a("Dana Whitfield", "dana.whitfield@gmail.test", "dana"),
            vec![me()],
            vec![],
            "Book club picks",
        );
        let t = b.thread();
        b.push(
            "example.com",
            Some(t),
            None,
            ts(2025, 9, 8, 18, 0),
            me(),
            vec![a("Dana Whitfield", "danawhitfield@gmail.test", "dana")],
            vec![],
            "Book club: my list",
        );
        let t = b.thread();
        b.push(
            "googlemail.test",
            Some(t),
            None,
            ts(2025, 10, 2, 18, 0),
            a(
                "Dana Whitfield",
                "Dana.Whitfield+lists@googlemail.test",
                "dana",
            ),
            vec![me()],
            vec![],
            "Book club: October",
        );
    }
    // Pat: corporate plus addressing.
    {
        let t = b.thread();
        b.push(
            "fabrikam.example",
            Some(t),
            None,
            ts(2026, 3, 3, 9, 0),
            a("Pat Moreno", "pat+hiring@fabrikam.example", "pat"),
            vec![me()],
            vec![],
            "Open roles at Fabrikam",
        );
        let t = b.thread();
        let m1 = b.push(
            "fabrikam.example",
            Some(t),
            None,
            ts(2026, 3, 20, 9, 0),
            a("Pat Moreno", "pat@fabrikam.example", "pat"),
            vec![me()],
            vec![],
            "Following up",
        );
        b.push(
            "example.com",
            Some(t),
            Some(m1),
            ts(2026, 3, 20, 12, 0),
            me(),
            vec![a("Pat Moreno", "pat@fabrikam.example", "pat")],
            vec![],
            "Re: Following up",
        );
    }
    // Alex Kim at contoso (common-name collision with the LinkedIn Alex Kim).
    {
        let alex = || a("Alex Kim", "alex.kim@contoso.example", "alex_ct");
        let t = b.thread();
        let m1 = b.push(
            "contoso.example",
            Some(t),
            None,
            ts(2026, 1, 12, 14, 0),
            alex(),
            vec![me()],
            vec![],
            "Contoso security review",
        );
        b.push(
            "example.com",
            Some(t),
            Some(m1),
            ts(2026, 1, 12, 16, 0),
            me(),
            vec![alex()],
            vec![],
            "Re: Contoso security review",
        );
    }
    // Mira: one address from her vCard.
    {
        let mira = || a("Mira Castellanos", "mira@tailspin.example", "mira");
        let t = b.thread();
        let m1 = b.push(
            "tailspin.example",
            Some(t),
            None,
            ts(2026, 7, 1, 10, 0),
            mira(),
            vec![me()],
            vec![],
            "Tailspin launch party",
        );
        b.push(
            "example.com",
            Some(t),
            Some(m1),
            ts(2026, 7, 1, 13, 0),
            me(),
            vec![mira()],
            vec![],
            "Re: Tailspin launch party",
        );
    }
    // Kofi: RFC 2047 display name with an emoji, encoded subject.
    {
        let kofi = || a("Kofi Mensah", "kofi@tailspin.example", "kofi");
        let t = b.thread();
        let id = b.push(
            "tailspin.example",
            Some(t),
            None,
            ts(2026, 8, 18, 8, 0),
            kofi(),
            vec![me()],
            vec![],
            "=?UTF-8?B?RGV2dG9vbHMgZGlubmVyIOKAkyBUaHVyc2RheQ==?=",
        );
        if let Some(last) = b.mails.last_mut() {
            last.from_header =
                Some("=?UTF-8?Q?Kofi_Mensah_=F0=9F=9A=80?= <kofi@tailspin.example>".into());
        }
        b.push(
            "example.com",
            Some(t),
            Some(id),
            ts(2026, 8, 18, 9, 0),
            me(),
            vec![kofi()],
            vec![],
            "Re: Devtools dinner – Thursday",
        );
    }
    // Elena: "Last, First" display name.
    {
        let elena = || a("Voss, Elena", "elena@tailspin.example", "elena");
        let t = b.thread();
        let m1 = b.push(
            "tailspin.example",
            Some(t),
            None,
            ts(2026, 5, 20, 9, 0),
            elena(),
            vec![me()],
            vec![],
            "Design review slots",
        );
        let m2 = b.push(
            "example.com",
            Some(t),
            Some(m1),
            ts(2026, 5, 20, 10, 0),
            me(),
            vec![elena()],
            vec![],
            "Re: Design review slots",
        );
        b.push(
            "tailspin.example",
            Some(t),
            Some(m2),
            ts(2026, 5, 20, 11, 0),
            elena(),
            vec![me()],
            vec![],
            "Re: Design review slots",
        );
    }
    // Ravi: two-way; the reply is duplicated verbatim below.
    let ravi_dup;
    {
        let ravi = || a("Ravi Iyer", "ravi@contoso.example", "ravi");
        let t = b.thread();
        let m1 = b.push(
            "contoso.example",
            Some(t),
            None,
            ts(2026, 9, 1, 9, 0),
            ravi(),
            vec![me()],
            vec![],
            "Board deck draft",
        );
        b.push(
            "example.com",
            Some(t),
            Some(m1),
            ts(2026, 9, 1, 11, 0),
            me(),
            vec![ravi()],
            vec![],
            "Re: Board deck draft",
        );
        let t = b.thread();
        let m3 = b.push(
            "contoso.example",
            Some(t),
            None,
            ts(2026, 9, 15, 9, 0),
            ravi(),
            vec![me()],
            vec![],
            "Pricing page copy",
        );
        b.push(
            "example.com",
            Some(t),
            Some(m3),
            ts(2026, 9, 15, 10, 0),
            me(),
            vec![ravi()],
            vec![],
            "Re: Pricing page copy",
        );
        ravi_dup = b.mails.last().cloned();
    }
    // Nora: one cold inbound message.
    {
        let t = b.thread();
        b.push(
            "fabrikam.example",
            Some(t),
            None,
            ts(2026, 6, 30, 7, 0),
            a("Nora Lindqvist", "nora@fabrikam.example", "nora"),
            vec![me()],
            vec![],
            "Quick question about your API",
        );
    }
    // Hannah: RFC 2047 quoted-printable display name.
    {
        let hannah = || a("Hannah O'Neil", "hannah@contoso.example", "hannah");
        let t = b.thread();
        let m1 = b.push(
            "contoso.example",
            Some(t),
            None,
            ts(2026, 2, 25, 9, 0),
            hannah(),
            vec![me()],
            vec![],
            "Research interviews",
        );
        if let Some(last) = b.mails.last_mut() {
            last.from_header = Some("=?UTF-8?Q?Hannah_O'Neil?= <hannah@contoso.example>".into());
        }
        b.push(
            "example.com",
            Some(t),
            Some(m1),
            ts(2026, 2, 25, 12, 0),
            me(),
            vec![hannah()],
            vec![],
            "Re: Research interviews",
        );
    }
    // Digest Weekly: 60 newsletter issues with List-Unsubscribe.
    for i in 0..60 {
        let t = b.thread();
        let at = ts(2025, 6, 2, 6, 0) + i * 7 * 86_400;
        b.push(
            "digest.example",
            Some(t),
            None,
            at,
            a("Digest Weekly", "news@digest.example", "digest"),
            vec![me()],
            vec![],
            &format!("Digest Weekly issue {}", i + 1),
        );
        if let Some(last) = b.mails.last_mut() {
            last.list_unsub = true;
            last.labels = "Inbox,Category Promotions";
            last.undisclosed = i == 0;
        }
    }
    // Malformed: no usable date anywhere -> skipped (bad_date).
    {
        let t = b.thread();
        b.push(
            "example.invalid",
            Some(t),
            None,
            0,
            a("Ghost Sender", "ghost@example.invalid", "-"),
            vec![me()],
            vec![],
            "This message has no valid date",
        );
        if let Some(last) = b.mails.last_mut() {
            last.fate = MailFate::SkipBadDate;
        }
    }
    // Spam-labelled -> skipped (unsupported).
    {
        let t = b.thread();
        b.push(
            "spam.example",
            Some(t),
            None,
            ts(2026, 9, 20, 3, 0),
            a("Prize Desk", "prizes@spam.example", "-"),
            vec![me()],
            vec![],
            "You have won",
        );
        if let Some(last) = b.mails.last_mut() {
            last.labels = "Spam";
            last.fate = MailFate::SkipSpam;
        }
    }
    // Duplicate of Ravi's last reply (same Message-ID) -> interactions_dup.
    if let Some(mut dup) = ravi_dup {
        dup.fate = MailFate::Duplicate;
        b.mails.push(dup);
    }
    b.mails
}

fn addr_header(list: &[Addr]) -> String {
    list.iter()
        .map(|(n, e, _)| {
            if n.contains(',') || n.contains('\'') {
                format!("\"{n}\" <{e}>")
            } else {
                format!("{n} <{e}>")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The mbox file (headers + small bodies containing the sentinel).
pub fn mbox_text(mails: &[Mail]) -> String {
    let mut out = String::new();
    for (i, mm) in mails.iter().enumerate() {
        match mm.fate {
            MailFate::SkipBadDate => {
                let _ = writeln!(out, "From {} ???", mm.from.1);
            }
            _ => {
                let _ = writeln!(out, "From {} {}", mm.from.1, from_line_date(mm.at));
            }
        }
        if let Some(t) = mm.thrid {
            let _ = writeln!(out, "X-GM-THRID: {t}");
        }
        let _ = writeln!(out, "X-Gmail-Labels: {}", mm.labels);
        let _ = writeln!(out, "Message-ID: <{}>", mm.id);
        match mm.fate {
            MailFate::SkipBadDate => {
                let _ = writeln!(out, "Date: not a date");
            }
            _ => {
                let _ = writeln!(out, "Date: {}", rfc2822(mm.at));
            }
        }
        match &mm.from_header {
            Some(h) => {
                let _ = writeln!(out, "From: {h}");
            }
            None => {
                let _ = writeln!(out, "From: {}", addr_header(std::slice::from_ref(&mm.from)));
            }
        }
        if mm.undisclosed {
            let _ = writeln!(out, "To: undisclosed-recipients:;");
        } else {
            let _ = writeln!(out, "To: {}", addr_header(&mm.to));
        }
        if !mm.cc.is_empty() {
            let _ = writeln!(out, "Cc: {}", addr_header(&mm.cc));
        }
        let _ = writeln!(out, "Subject: {}", mm.subject);
        if let Some(r) = &mm.reply_to {
            let _ = writeln!(out, "In-Reply-To: <{r}>");
            let _ = writeln!(out, "References: <{r}>");
        }
        if mm.list_unsub {
            let _ = writeln!(
                out,
                "List-Unsubscribe: <https://digest.example/unsubscribe>"
            );
            let _ = writeln!(out, "List-Id: Digest Weekly <weekly.digest.example>");
        }
        let _ = writeln!(out, "MIME-Version: 1.0");
        let _ = writeln!(out, "Content-Type: text/plain; charset=UTF-8");
        let _ = writeln!(out);
        let _ = writeln!(out, "Hi, this is a synthetic fixture body. {BODY_SENTINEL}");
        if i % 10 == 0 {
            // mboxrd escaping and a header-looking body line: neither may be
            // read as a header or a message boundary.
            let _ = writeln!(out, ">From the archive: an escaped body line.");
            let _ = writeln!(out, "From: someone-in-a-body@example.invalid");
        }
        let _ = writeln!(out, "-- ");
        let _ = writeln!(out, "Signature line {BODY_SENTINEL}");
        let _ = writeln!(out);
    }
    out
}

/// One LinkedIn connection row.
#[derive(Debug, Clone)]
pub struct Connection {
    /// Cast key.
    pub key: &'static str,
    /// First Name.
    pub first: &'static str,
    /// Last Name.
    pub last: &'static str,
    /// Vanity slug.
    pub slug: &'static str,
    /// Email Address (often empty).
    pub email: &'static str,
    /// Company.
    pub company: &'static str,
    /// Position.
    pub position: &'static str,
    /// Connected On.
    pub at: i64,
}

/// `Connections.csv` rows.
pub fn connections() -> Vec<Connection> {
    let c = |key, first, last, slug, email, company, position, at| Connection {
        key,
        first,
        last,
        slug,
        email,
        company,
        position,
        at,
    };
    vec![
        c(
            "priya",
            "Priya",
            "Raman",
            "priya-raman-nw",
            "priya@northwind.example",
            "Northwind",
            "VP Engineering",
            ts(2024, 3, 12, 0, 0),
        ),
        c(
            "marcus",
            "Marcus",
            "Lee",
            "marcus-lee-contoso",
            "marcus@contoso.example",
            "Contoso",
            "Staff Engineer",
            ts(2023, 11, 2, 0, 0),
        ),
        c(
            "sam",
            "Sam",
            "Ito",
            "sam-ito-nw",
            "",
            "Northwind",
            "Product Manager",
            ts(2025, 5, 19, 0, 0),
        ),
        c(
            "lena",
            "Lena",
            "Park",
            "lena-park-design",
            "",
            "Contoso",
            "Design Lead",
            ts(2024, 8, 7, 0, 0),
        ),
        c(
            "alex_nw",
            "Alex",
            "Kim",
            "alex-kim-northwind",
            "",
            "Northwind",
            "Software Engineer",
            ts(2025, 2, 14, 0, 0),
        ),
        c(
            "ines_li",
            "Ines",
            "Quillfeather",
            "ines-quillfeather",
            "",
            "Halcyon",
            "Founder",
            ts(2024, 1, 30, 0, 0),
        ),
        c(
            "john",
            "John",
            "Quillby",
            "john-quillby",
            "",
            "Brightwater",
            "CTO",
            ts(2025, 7, 7, 0, 0),
        ),
        c(
            "chidi",
            "Chidi",
            "Okafor",
            "chidi-okafor",
            "",
            "Fabrikam",
            "Technical Recruiter",
            ts(2025, 9, 9, 0, 0),
        ),
        c(
            "sofia",
            "Sofia",
            "Marchetti",
            "sofia-marchetti",
            "",
            "Tailspin",
            "Partner, Seed Fund",
            ts(2024, 10, 15, 0, 0),
        ),
        c(
            "kenji",
            "Kenji",
            "Watanabe",
            "kenji-watanabe-nw",
            "",
            "Northwind",
            "Co-founder",
            ts(2023, 6, 1, 0, 0),
        ),
        c(
            "amara",
            "Amara",
            "Nwosu",
            "amara-nwosu",
            "",
            "Contoso",
            "Head of Sales",
            ts(2025, 12, 1, 0, 0),
        ),
        c(
            "jose",
            "Jose",
            "Garcia",
            "jose-garcia-tailspin",
            "",
            "Tailspin",
            "Data Scientist",
            ts(2025, 4, 22, 0, 0),
        ),
    ]
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn linkedin_url(slug: &str) -> String {
    format!("https://www.linkedin.com/in/{slug}")
}

/// `Connections.csv`: UTF-8 BOM, `Notes:` preamble, CRLF line endings.
pub fn connections_csv(rows: &[Connection]) -> String {
    let mut out = String::from("\u{feff}Notes:\r\n");
    out.push_str("\"When exporting your connection data, you may notice that some of the email addresses are missing. You will only see email addresses for connections who have allowed their connections to see or download their email address.\"\r\n");
    out.push_str("\r\n");
    out.push_str("First Name,Last Name,URL,Email Address,Company,Position,Connected On\r\n");
    for r in rows {
        let fields = [
            r.first.to_string(),
            r.last.to_string(),
            linkedin_url(r.slug),
            r.email.to_string(),
            r.company.to_string(),
            r.position.to_string(),
            linkedin_date(r.at),
        ];
        let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

/// One LinkedIn DM row: (conversation, title, from name, from slug, to name, to slug, at, subject).
#[derive(Debug, Clone)]
pub struct Dm {
    /// CONVERSATION ID.
    pub conversation: &'static str,
    /// CONVERSATION TITLE.
    pub title: &'static str,
    /// FROM.
    pub from: &'static str,
    /// Sender slug.
    pub from_slug: &'static str,
    /// TO.
    pub to: &'static str,
    /// Recipient slug.
    pub to_slug: &'static str,
    /// DATE.
    pub at: i64,
    /// SUBJECT (often empty).
    pub subject: &'static str,
    /// FOLDER.
    pub folder: &'static str,
    /// Cast key of the non-owner side.
    pub other: &'static str,
}

/// `messages.csv` rows.
pub fn dms() -> Vec<Dm> {
    let conv_p = "2-ZmFrZS1wcml5YS1jb252";
    let conv_h = "2-ZmFrZS1odWdvLWNvbnY=";
    let conv_a = "2-ZmFrZS1hbWFyYS1jb252";
    vec![
        Dm {
            conversation: conv_p,
            title: "",
            from: OWNER_NAME,
            from_slug: OWNER_SLUG,
            to: "Priya Raman",
            to_slug: "priya-raman-nw",
            at: ts(2026, 8, 3, 10, 0),
            subject: "",
            folder: "SENT",
            other: "priya",
        },
        Dm {
            conversation: conv_p,
            title: "",
            from: "Priya Raman",
            from_slug: "priya-raman-nw",
            to: OWNER_NAME,
            to_slug: OWNER_SLUG,
            at: ts(2026, 8, 3, 12, 30),
            subject: "",
            folder: "INBOX",
            other: "priya",
        },
        Dm {
            conversation: conv_h,
            title: "Speaking slot",
            from: "Hugo Brandt",
            from_slug: "hugo-brandt",
            to: OWNER_NAME,
            to_slug: OWNER_SLUG,
            at: ts(2026, 4, 2, 16, 0),
            subject: "Speaking slot at Devtools Days",
            folder: "INBOX",
            other: "hugo",
        },
        Dm {
            conversation: conv_a,
            title: "",
            from: "Amara Nwosu",
            from_slug: "amara-nwosu",
            to: OWNER_NAME,
            to_slug: OWNER_SLUG,
            at: ts(2026, 1, 20, 9, 0),
            subject: "",
            folder: "INBOX",
            other: "amara",
        },
        Dm {
            conversation: conv_a,
            title: "",
            from: OWNER_NAME,
            from_slug: OWNER_SLUG,
            to: "Amara Nwosu",
            to_slug: "amara-nwosu",
            at: ts(2026, 1, 21, 9, 0),
            subject: "",
            folder: "SENT",
            other: "amara",
        },
    ]
}

/// `messages.csv` (LF line endings; CONTENT carries the sentinel).
pub fn messages_csv(rows: &[Dm]) -> String {
    let mut out = String::from(
        "CONVERSATION ID,CONVERSATION TITLE,FROM,SENDER PROFILE URL,TO,RECIPIENT PROFILE URLS,DATE,SUBJECT,CONTENT,FOLDER\n",
    );
    for r in rows {
        let content =
            format!("Message text, never stored. {BODY_SENTINEL}\nSecond line of the message.");
        let fields = [
            r.conversation.to_string(),
            r.title.to_string(),
            r.from.to_string(),
            linkedin_url(r.from_slug),
            r.to.to_string(),
            linkedin_url(r.to_slug),
            linkedin_datetime(r.at),
            r.subject.to_string(),
            content,
            r.folder.to_string(),
        ];
        let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// One invitation row.
#[derive(Debug, Clone)]
pub struct Invitation {
    /// From (name).
    pub from: &'static str,
    /// To (name).
    pub to: &'static str,
    /// Sent At.
    pub at: i64,
    /// INCOMING / OUTGOING.
    pub direction: &'static str,
    /// Inviter slug.
    pub inviter: &'static str,
    /// Invitee slug.
    pub invitee: &'static str,
    /// Cast key of the non-owner side.
    pub other: &'static str,
}

/// `Invitations.csv` rows.
pub fn invitations() -> Vec<Invitation> {
    vec![
        Invitation {
            from: OWNER_NAME,
            to: "Greta Solberg",
            at: ts(2026, 9, 15, 10, 0),
            direction: "OUTGOING",
            inviter: OWNER_SLUG,
            invitee: "greta-solberg",
            other: "greta",
        },
        Invitation {
            from: "Lena Park",
            to: OWNER_NAME,
            at: ts(2024, 8, 6, 15, 45),
            direction: "INCOMING",
            inviter: "lena-park-design",
            invitee: OWNER_SLUG,
            other: "lena",
        },
    ]
}

/// `Invitations.csv` (Message carries the sentinel).
pub fn invitations_csv(rows: &[Invitation]) -> String {
    let mut out =
        String::from("From,To,Sent At,Message,Direction,inviterProfileUrl,inviteeProfileUrl\n");
    for r in rows {
        let fields = [
            r.from.to_string(),
            r.to.to_string(),
            linkedin_sent_at(r.at),
            format!("Invitation note, never stored. {BODY_SENTINEL}"),
            r.direction.to_string(),
            linkedin_url(r.inviter),
            linkedin_url(r.invitee),
        ];
        let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// One vCard (already rendered) and the handles it carries.
#[derive(Debug, Clone)]
pub struct Card {
    /// Cast key.
    pub key: &'static str,
    /// Normalized handles (`email:...`, `phone:+1...`).
    pub handles: Vec<String>,
    /// Rendered card with CRLF line endings.
    pub text: String,
}

fn card(key: &'static str, handles: &[&str], lines: &[&str]) -> Card {
    let mut text = String::new();
    for l in lines {
        text.push_str(l);
        text.push_str("\r\n");
    }
    Card {
        key,
        handles: handles.iter().map(|h| h.to_string()).collect(),
        text,
    }
}

/// The vCards, in file order.
pub fn cards() -> Vec<Card> {
    let rev = |t| format!("REV:{}", vcard_rev(t));
    let r1 = rev(ts(2026, 3, 1, 12, 0));
    let r2 = rev(ts(2025, 11, 20, 8, 30));
    vec![
        card(
            "priya",
            &["email:priya@northwind.example", "phone:+12015550142"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "PRODID:-//Apple Inc.//macOS 15.0//EN",
                "N:Raman;Priya;;;",
                "FN:Priya Raman",
                "ORG:Northwind;Platform",
                "TITLE:VP Engineering",
                "EMAIL;type=INTERNET;type=WORK;type=pref:priya@northwind.example",
                "TEL;type=CELL;type=VOICE;type=pref:+1 (201) 555-0142",
                "ADR;type=WORK:;;1 Example Road;Bengaluru;KA;560001;India",
                &format!("NOTE:Met at a conference. {BODY_SENTINEL}"),
                &r1,
                "UID:8F1A0C44-0000-4000-8000-000000000001",
                "END:VCARD",
            ],
        ),
        card(
            "lena",
            &["email:lena.park@mail.test"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:Park;Lena;;;",
                "FN:Lena Park",
                "ORG:Contoso",
                "TITLE:Designer",
                "EMAIL;type=INTERNET;type=HOME:lena.park@mail.test",
                "UID:8F1A0C44-0000-4000-8000-000000000002",
                "END:VCARD",
            ],
        ),
        card(
            "ines_vcf",
            &["email:ines.q@mail.test"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:Quillfeather;Ines;;;",
                "FN:Ines Quillfeather",
                "EMAIL;type=INTERNET:ines.q@mail.test",
                "UID:8F1A0C44-0000-4000-8000-000000000003",
                "END:VCARD",
            ],
        ),
        card(
            "jon",
            &["email:jon.quillby@brightwater.example"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:Quillby;Jon;;;",
                "FN:Jon Quillby",
                "ORG:Brightwater",
                "EMAIL;type=INTERNET;type=WORK:jon.quillby@brightwater.example",
                "UID:8F1A0C44-0000-4000-8000-000000000004",
                "END:VCARD",
            ],
        ),
        card(
            "rufus",
            &["phone:+12015550177"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:Pennyworth;Rufus;;;",
                "FN:Rufus Pennyworth",
                "TEL;type=CELL:+1 201-555-0177",
                "UID:8F1A0C44-0000-4000-8000-000000000005",
                "END:VCARD",
            ],
        ),
        card(
            "mira",
            &[
                "email:mira@tailspin.example",
                "email:mira.c@mail.test",
                "phone:+12015550188",
            ],
            &[
                "BEGIN:VCARD",
                "VERSION:4.0",
                "FN:Mira Castellanos",
                "N:Castellanos;Mira;;;",
                "ORG:Tailspin",
                "TITLE:Head of Marketing",
                "EMAIL;TYPE=work:mira@tailspin.example",
                "EMAIL;TYPE=home:mira.c@mail.test",
                "TEL;VALUE=uri;TYPE=cell:tel:+1-201-555-0188",
                "ADR;TYPE=work:;;;Lisbon;;;Portugal",
                &r2,
                "UID:urn:uuid:8f1a0c44-0000-4000-8000-000000000006",
                "END:VCARD",
            ],
        ),
        card(
            "kofi",
            &["email:kofi@tailspin.example"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:Mensah;Kofi;;;",
                "FN:Kofi Mensah",
                "ORG:Tailspin",
                "EMAIL;type=INTERNET:kofi@tailspin.example",
                "PHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAP////////////",
                " ////////////////////////////////////////////////////////////wgALCAABAAEB",
                "UID:8F1A0C44-0000-4000-8000-000000000007",
                "END:VCARD",
            ],
        ),
        card(
            "beatriz",
            &["email:beatriz@mail.test"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:Okonkwo;Beatriz;;;",
                "FN:Beatriz Okonkwo",
                "EMAIL;type=INTERNET:beatriz@mail.test",
                "CATEGORIES:friends",
                "UID:8F1A0C44-0000-4000-8000-000000000008",
                "END:VCARD",
            ],
        ),
        card(
            "lars",
            &["email:lars@tailspin.example"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:Eriksen;Lars;;;",
                "FN:Lars Eriksen",
                "ORG:Tailspin",
                "item1.EMAIL;type=INTERNET:lars@tailspin.example",
                "item1.X-ABLabel:work",
                "UID:8F1A0C44-0000-4000-8000-000000000009",
                "END:VCARD",
            ],
        ),
        card(
            "jose",
            &["email:jose.garcia@tailspin.example"],
            &[
                "BEGIN:VCARD",
                "VERSION:2.1",
                "N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Garc=C3=ADa;Jos=C3=A9;;;",
                "FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Jos=C3=A9 Garc=C3=ADa",
                "EMAIL;INTERNET:jose.garcia@tailspin.example",
                "UID:8F1A0C44-0000-4000-8000-000000000010",
                "END:VCARD",
            ],
        ),
        card(
            "hannah",
            &["email:hannah@contoso.example"],
            &[
                "BEGIN:VCARD",
                "VERSION:3.0",
                "N:O'Neil;Hannah;;Dr.;",
                "FN:Dr. Hannah O'Neil",
                "ORG:Contoso",
                "TITLE:Research Lead",
                "EMAIL;type=INTERNET;type=WORK:",
                " hannah@contoso.example",
                "UID:8F1A0C44-0000-4000-8000-000000000011",
                "END:VCARD",
            ],
        ),
    ]
}

/// `contacts.vcf`.
pub fn vcf_text(cards: &[Card]) -> String {
    cards.iter().map(|c| c.text.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Expected totals, computed from the records and the cast annotations
// ---------------------------------------------------------------------------

/// Totals the resolution tests must hit exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    /// `person` rows including the self person.
    pub persons: usize,
    /// `person` rows excluding self.
    pub persons_excluding_self: usize,
    /// People with flagged identities.
    pub flagged: usize,
    /// Review suggestions.
    pub suggestions: usize,
    /// Bulk senders.
    pub bulk: usize,
    /// Distinct normalized handles (`identity` rows).
    pub identities: usize,
    /// Email interactions.
    pub emails: usize,
    /// LinkedIn connection + invitation interactions.
    pub connections: usize,
    /// LinkedIn DM interactions.
    pub dms: usize,
    /// vCard contact interactions.
    pub contacts: usize,
    /// mbox messages read.
    pub mbox_messages: usize,
    /// mbox skips by reason.
    pub mbox_skipped: BTreeMap<&'static str, usize>,
    /// mbox duplicates.
    pub mbox_dups: usize,
}

fn add_handle(map: &mut BTreeMap<String, &'static str>, handle: String, key: &'static str) {
    if let Some(prev) = map.insert(handle.clone(), key) {
        assert_eq!(prev, key, "handle {handle} belongs to two cast members");
    }
}

/// Every normalized handle the fixtures carry, mapped to its cast key.
pub fn handle_map() -> BTreeMap<String, &'static str> {
    let mut map = BTreeMap::new();
    for mm in mails() {
        if mm.fate != MailFate::Import {
            continue;
        }
        add_handle(
            &mut map,
            format!("email:{}", mm.from.1.to_lowercase()),
            mm.from.2,
        );
        if !mm.undisclosed {
            for x in &mm.to {
                add_handle(&mut map, format!("email:{}", x.1.to_lowercase()), x.2);
            }
        }
        for x in &mm.cc {
            add_handle(&mut map, format!("email:{}", x.1.to_lowercase()), x.2);
        }
    }
    for c in connections() {
        add_handle(&mut map, format!("linkedin_url:{}", c.slug), c.key);
        if !c.email.is_empty() {
            add_handle(&mut map, format!("email:{}", c.email), c.key);
        }
    }
    for d in dms() {
        let (from_key, to_key) = if d.from_slug == OWNER_SLUG {
            ("self", d.other)
        } else {
            (d.other, "self")
        };
        add_handle(&mut map, format!("linkedin_url:{}", d.from_slug), from_key);
        add_handle(&mut map, format!("linkedin_url:{}", d.to_slug), to_key);
    }
    for inv in invitations() {
        let (from_key, to_key) = if inv.inviter == OWNER_SLUG {
            ("self", inv.other)
        } else {
            (inv.other, "self")
        };
        add_handle(&mut map, format!("linkedin_url:{}", inv.inviter), from_key);
        add_handle(&mut map, format!("linkedin_url:{}", inv.invitee), to_key);
    }
    for c in cards() {
        for h in c.handles {
            add_handle(&mut map, h, c.key);
        }
    }
    map
}

/// Compute the expected totals.
pub fn expected() -> Expected {
    let handles = handle_map();
    let people: BTreeSet<&str> = handles.values().copied().collect();
    let cast_keys: BTreeSet<&str> = CAST.iter().map(|c| c.key).collect();
    assert_eq!(
        people, cast_keys,
        "every cast member appears in the fixtures and nobody else does"
    );
    let ms = mails();
    let mut skipped = BTreeMap::new();
    for mm in &ms {
        match mm.fate {
            MailFate::SkipBadDate => *skipped.entry("bad_date").or_insert(0) += 1,
            MailFate::SkipSpam => *skipped.entry("unsupported").or_insert(0) += 1,
            _ => {}
        }
    }
    Expected {
        persons: people.len(),
        persons_excluding_self: people.len() - 1,
        flagged: CAST
            .iter()
            .filter(|c| {
                matches!(
                    c.merge,
                    Merge::FlaggedNameCompany | Merge::FlaggedNameThread
                )
            })
            .count(),
        suggestions: SUGGESTIONS.len(),
        bulk: CAST.iter().filter(|c| c.bulk).count(),
        identities: handles.len(),
        emails: ms.iter().filter(|m| m.fate == MailFate::Import).count(),
        connections: connections().len() + invitations().len(),
        dms: dms().len(),
        contacts: cards().len(),
        mbox_messages: ms.len(),
        mbox_skipped: skipped,
        mbox_dups: ms.iter().filter(|m| m.fate == MailFate::Duplicate).count(),
    }
}

fn merge_label(m: Merge) -> &'static str {
    match m {
        Merge::Single => "single",
        Merge::Exact => "exact (1.0)",
        Merge::Alias => "gmail_alias (0.98)",
        Merge::FlaggedNameCompany => "**flagged** name_company (0.85)",
        Merge::FlaggedNameThread => "**flagged** name_thread (0.75)",
    }
}

/// `fixtures/EXPECTED.md`.
pub fn expected_md() -> String {
    let e = expected();
    let handles = handle_map();
    let mut out = String::new();
    out.push_str("# Fixture contract (Phase 1)\n\n");
    out.push_str("Generated by `cargo run -p openstance-fixtures --bin gen-fixtures`; do not edit by hand.\n");
    out.push_str("Every name, domain and phone number is fictional (`*.example`, `*.test`, `*.invalid`, `example.com`, `+1 201 555-01xx`).\n\n");
    out.push_str("**This is the contract the resolution stream must hit exactly (P1-CORE-12).** The numbers were derived by\n");
    out.push_str("walking every cast member through the rules in `docs/tech-spec/entity-resolution.md`; the reasoning for each\n");
    out.push_str("person is in the cast table below. Changing a fixture or a rule means updating this file deliberately.\n\n");
    out.push_str("## How to load\n\n");
    out.push_str("Import all five files into one graph **with `fixtures/config.toml`** (it sets the owner's `[me]` handles and\n");
    out.push_str("treats `gmail.test` / `googlemail.test` as Gmail domains and `mail.test` as freemail):\n\n");
    out.push_str("```sh\nOPENSTANCE_CONFIG=fixtures/config.toml openstance import \\\n  fixtures/mbox/small.mbox fixtures/linkedin/Connections.csv fixtures/linkedin/messages.csv \\\n  fixtures/linkedin/Invitations.csv fixtures/vcf/contacts.vcf\n```\n\n");
    out.push_str("Totals are independent of import order (incremental equals full rebuild).\n\n");
    out.push_str("## Totals\n\n| Quantity | Expected | Notes |\n|---|---|---|\n");
    let _ = writeln!(
        out,
        "| persons (all `person` rows) | **{}** | {} people + the self person |",
        e.persons, e.persons_excluding_self
    );
    let _ = writeln!(
        out,
        "| persons excluding self | **{}** | `is_self = 0` |",
        e.persons_excluding_self
    );
    let _ = writeln!(
        out,
        "| flagged merges | **{}** | people with an identity at `name_company` or `name_thread` |",
        e.flagged
    );
    let _ = writeln!(
        out,
        "| review suggestions | **{}** | never auto-merged |",
        e.suggestions
    );
    let _ = writeln!(
        out,
        "| bulk senders | **{}** | `is_bulk = 1`, score 0 |",
        e.bulk
    );
    let _ = writeln!(
        out,
        "| identities | **{}** | distinct normalized handles |",
        e.identities
    );
    let _ = writeln!(
        out,
        "| email interactions | {} | {} messages read, minus {} skipped and {} duplicate |",
        e.emails,
        e.mbox_messages,
        e.mbox_skipped.values().sum::<usize>(),
        e.mbox_dups
    );
    let _ = writeln!(
        out,
        "| connection interactions | {} | {} Connections.csv rows + {} Invitations.csv rows |",
        e.connections,
        connections().len(),
        invitations().len()
    );
    let _ = writeln!(out, "| dm interactions | {} | messages.csv rows |", e.dms);
    let _ = writeln!(
        out,
        "| contact interactions | {} | one per vCard |",
        e.contacts
    );
    let _ = writeln!(
        out,
        "| interactions excluding contact | {} | |",
        e.emails + e.connections + e.dms
    );
    out.push_str("\nmbox `ImportStats`: ");
    let _ = write!(out, "`records_read = {}`", e.mbox_messages);
    for (k, v) in &e.mbox_skipped {
        let _ = write!(out, ", `skipped.{k} = {v}`");
    }
    let _ = writeln!(
        out,
        ", `interactions_dup = {}`, `interactions_new = {}`.\n",
        e.mbox_dups, e.emails
    );
    out.push_str("## Must-hold assertions\n\n");
    out.push_str("- The common-name pair **Alex Kim** (LinkedIn at Northwind vs `alex.kim@contoso.example`) stays two people with no suggestion.\n");
    out.push_str("- The Gmail alias trio for **Dana Whitfield** is one person; two of its identities are `gmail_alias` at 0.98.\n");
    out.push_str("- **Pat Moreno** (`pat+hiring@` / `pat@fabrikam.example`) is one person via `gmail_alias` (corporate plus) at 0.98, not flagged.\n");
    out.push_str("- **Digest Weekly** (`news@digest.example`) is the only bulk sender; **Priya Raman** (52 inbound, 26 outbound, threaded) is not bulk.\n");
    out.push_str("- **Priya Raman** and **Marcus Lee** share exactly 3 email interactions: `tie(kind='email').n_shared = 3`, and the tie evidence query returns those 3. The self person never appears in `tie`.\n");
    out.push_str("- **Kofi Mensah**, **Priya Raman** and **Hannah O'Neil** merge mbox with vcf by email at confidence 1.0 (`exact`/`anchor`).\n");
    out.push_str(
        "- **Rufus Pennyworth** is a person whose only identity is `phone:+12015550177`.\n",
    );
    let _ = writeln!(out, "- No column of any table contains `{BODY_SENTINEL}` (bodies, DM content, invitation messages and vCard NOTEs are never stored).");
    out.push_str("- The malformed-date message and the Spam-labelled message create no identities (`ghost@example.invalid`, `prizes@spam.example` absent).\n\n");
    out.push_str("## Suggestions\n\n| a | b | rule | why |\n|---|---|---|---|\n");
    for s in SUGGESTIONS {
        let name = |k: &str| CAST.iter().find(|c| c.key == k).map_or("?", |c| c.name);
        let _ = writeln!(
            out,
            "| {} (`{}`) | {} (`{}`) | `{}` | {} |",
            name(s.a),
            s.a,
            name(s.b),
            s.b,
            s.rule,
            s.note
        );
    }
    out.push_str("\n## Cast\n\n| key | name | expected merge | bulk | handles | scenario |\n|---|---|---|---|---|---|\n");
    for c in CAST {
        let hs: Vec<String> = handles
            .iter()
            .filter(|(_, k)| **k == c.key)
            .map(|(h, _)| format!("`{h}`"))
            .collect();
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} |",
            c.key,
            c.name,
            merge_label(c.merge),
            if c.bulk { "yes" } else { "" },
            hs.join("<br>"),
            c.scenario
        );
    }
    out.push_str("\n## Formats exercised\n\n");
    out.push_str("- `mbox/small.mbox`: Takeout-style `X-GM-THRID` / `X-Gmail-Labels`, `Sent` labels for owner mail, one thread keyed only by `In-Reply-To`, RFC 2047 B- and Q-encoded headers (emoji, apostrophe), `\"Last, First\"` display names, `undisclosed-recipients:;`, `List-Unsubscribe`, a `>From ` escaped body line, a header-looking body line, one message with no valid date (`From ` line and `Date` both broken), one `Spam` message, one duplicated Message-ID.\n");
    out.push_str("- `linkedin/Connections.csv`: UTF-8 BOM, `Notes:` preamble, CRLF, empty `Email Address` for most rows, a quoted comma in `Position`, `Connected On` as `12 Mar 2024`.\n");
    out.push_str("- `linkedin/messages.csv`: vanity profile URLs, `DATE` as `2026-08-03 10:00:00 UTC`, multi-line quoted `CONTENT` (never read), `FOLDER` INBOX/SENT.\n");
    out.push_str("- `linkedin/Invitations.csv`: `Sent At` as `9/15/26, 10:00 AM`, INCOMING/OUTGOING, `Message` never read.\n");
    out.push_str("- `vcf/contacts.vcf`: vCard 2.1 (quoted-printable UTF-8), 3.0 and 4.0 (`tel:` URI), CRLF, folded lines (base64 PHOTO, folded EMAIL value), grouped `item1.EMAIL`, `ADR`, `REV`, `NOTE`/`CATEGORIES` ignored.\n");
    out
}

/// The config every fixture test must load (owner handles, test Gmail domains).
pub fn config_toml() -> String {
    format!(
        "# Config for the synthetic fixtures. Load with OPENSTANCE_CONFIG=fixtures/config.toml.\n\
         [me]\n\
         identities = [\"email:{OWNER_EMAIL}\", \"linkedin_url:{OWNER_SLUG}\"]\n\
         \n\
         [resolution]\n\
         gmail_domains = [\"gmail.test\", \"googlemail.test\"]\n\
         freemail_extra = [\"gmail.test\", \"googlemail.test\", \"mail.test\"]\n"
    )
}

/// Every generated file: relative path -> contents.
pub fn files() -> BTreeMap<&'static str, String> {
    let mut f = BTreeMap::new();
    f.insert("mbox/small.mbox", mbox_text(&mails()));
    f.insert("linkedin/Connections.csv", connections_csv(&connections()));
    f.insert("linkedin/messages.csv", messages_csv(&dms()));
    f.insert("linkedin/Invitations.csv", invitations_csv(&invitations()));
    f.insert("vcf/contacts.vcf", vcf_text(&cards()));
    f.insert("config.toml", config_toml());
    f.insert("EXPECTED.md", expected_md());
    f
}

/// Write every fixture file under `root` (the repository's `fixtures/`).
pub fn generate(root: &Path) -> std::io::Result<()> {
    for (rel, text) in files() {
        let path = root.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_helpers_are_correct() {
        assert_eq!(ts(1970, 1, 1, 0, 0), 0);
        assert_eq!(ts(2026, 10, 1, 0, 0), 1_790_812_800);
        assert_eq!(
            rfc2822(ts(2025, 1, 6, 9, 0)),
            "Mon, 06 Jan 2025 09:00:00 +0000"
        );
        assert_eq!(
            from_line_date(ts(2025, 1, 6, 9, 0)),
            "Mon Jan  6 09:00:00 2025"
        );
        assert_eq!(linkedin_date(ts(2024, 3, 12, 0, 0)), "12 Mar 2024");
        assert_eq!(
            linkedin_sent_at(ts(2026, 9, 15, 10, 0)),
            "9/15/26, 10:00 AM"
        );
        assert_eq!(linkedin_sent_at(ts(2024, 8, 6, 15, 45)), "8/6/24, 3:45 PM");
        assert_eq!(vcard_rev(ts(2026, 3, 1, 12, 0)), "20260301T120000Z");
    }

    #[test]
    fn expected_totals_are_the_documented_contract() {
        let e = expected();
        assert_eq!(e.persons, 31);
        assert_eq!(e.persons_excluding_self, 30);
        assert_eq!(e.flagged, 4);
        assert_eq!(e.suggestions, 2);
        assert_eq!(e.bulk, 1);
        assert_eq!(e.identities, 44);
    }

    #[test]
    fn fixtures_use_only_reserved_domains_and_fictional_phones() {
        for (h, _) in handle_map() {
            if let Some(email) = h.strip_prefix("email:") {
                let domain = email.rsplit('@').next().unwrap();
                assert!(
                    domain == "example.com"
                        || domain.ends_with(".example")
                        || domain.ends_with(".test")
                        || domain.ends_with(".invalid"),
                    "{h}"
                );
            }
            if let Some(phone) = h.strip_prefix("phone:") {
                assert!(phone.starts_with("+1201555"), "{h}");
            }
        }
    }

    #[test]
    fn sentinel_is_planted_in_every_free_text_field() {
        let f = files();
        for name in [
            "mbox/small.mbox",
            "linkedin/messages.csv",
            "linkedin/Invitations.csv",
            "vcf/contacts.vcf",
        ] {
            assert!(f[name].contains(BODY_SENTINEL), "{name}");
        }
    }

    #[test]
    fn files_are_small() {
        for (name, text) in files() {
            assert!(text.len() < 200 * 1024, "{name} is {} bytes", text.len());
        }
    }
}
