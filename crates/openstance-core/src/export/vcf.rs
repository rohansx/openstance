//! `openstance export --format vcf` (P3-CLI-05): one vCard 3.0 per person.

use std::io::Write;

use crate::error::Result;
use crate::json::PersonDetail;

/// Escape a vCard text value (RFC 2426 section 4: `\\`, `,`, `;`, newlines).
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => o.push_str("\\\\"),
            ',' => o.push_str("\\,"),
            ';' => o.push_str("\\;"),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            c if c.is_control() => {}
            c => o.push(c),
        }
    }
    o
}

/// Fold a content line at 75 octets (CRLF + space), never inside a UTF-8
/// sequence, and terminate it with CRLF.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut width = 0;
    for ch in line.chars() {
        let n = ch.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += n;
    }
    out.push_str("\r\n");
    out
}

/// Write one vCard.
pub fn write_card(out: &mut dyn Write, d: &PersonDetail) -> Result<()> {
    let p = &d.person;
    let mut lines: Vec<String> = vec!["BEGIN:VCARD".into(), "VERSION:3.0".into()];
    let name = p.display_name.as_deref().unwrap_or("").trim();
    let fallback = d
        .identities
        .iter()
        .find(|i| i.kind == "email")
        .map(|i| i.value.as_str())
        .unwrap_or("");
    let shown = if name.is_empty() { fallback } else { name };
    lines.push(format!("FN:{}", esc(shown)));
    let (given, family) = match name.rsplit_once(char::is_whitespace) {
        Some((g, f)) => (g.trim(), f.trim()),
        None => ("", name),
    };
    lines.push(format!("N:{};{};;;", esc(family), esc(given)));
    if let Some(org) = p.company.as_deref().filter(|s| !s.trim().is_empty()) {
        lines.push(format!("ORG:{}", esc(org)));
    }
    if let Some(t) = p.title.as_deref().filter(|s| !s.trim().is_empty()) {
        lines.push(format!("TITLE:{}", esc(t)));
    }
    for i in &d.identities {
        match i.kind.as_str() {
            "email" => lines.push(format!("EMAIL;TYPE=INTERNET:{}", esc(&i.value))),
            "phone" => lines.push(format!("TEL:{}", esc(&i.value))),
            "linkedin_url" => lines.push(format!(
                "URL:https://www.linkedin.com/in/{}",
                esc(i.value.trim_start_matches("in/"))
            )),
            "x_handle" => lines.push(format!(
                "X-SOCIALPROFILE;TYPE=twitter:https://x.com/{}",
                esc(i.value.trim_start_matches('@'))
            )),
            _ => {}
        }
    }
    if let Some(loc) = p.location.as_deref().filter(|s| !s.trim().is_empty()) {
        lines.push(format!("ADR;TYPE=WORK:;;;{};;;", esc(loc)));
    }
    if !p.tags.is_empty() {
        let tags: Vec<String> = p.tags.iter().map(|t| esc(t)).collect();
        lines.push(format!("CATEGORIES:{}", tags.join(",")));
    }
    lines.push(format!("UID:openstance-person-{}", p.id));
    lines.push("END:VCARD".into());
    let mut buf = String::new();
    for l in &lines {
        buf.push_str(&fold(l));
    }
    out.write_all(buf.as_bytes())
        .map_err(|e| crate::error::Error::io("writing", "export output", e))
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::export::{export, ExportFormat, ExportOptions};
    use crate::search::testkit::{indexed, NOW};

    fn vcf(opts: &ExportOptions) -> String {
        let c = indexed();
        let mut out = Vec::new();
        export(
            &c,
            &Config::default(),
            ExportFormat::Vcf,
            opts,
            &mut out,
            NOW,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn writes_one_card_per_person_with_crlf_and_identities() {
        let text = vcf(&ExportOptions::default());
        assert_eq!(text.matches("BEGIN:VCARD\r\n").count(), 5, "{text}");
        assert_eq!(text.matches("END:VCARD\r\n").count(), 5);
        assert!(text.contains("VERSION:3.0\r\n"));
        assert!(text.contains("FN:Priya Raman\r\n"));
        assert!(text.contains("N:Raman;Priya;;;\r\n"));
        assert!(text.contains("ORG:Northwind\r\n"));
        assert!(text.contains("TITLE:Co-founder & CEO\r\n"));
        assert!(text.contains("EMAIL;TYPE=INTERNET:priya@northwind.example\r\n"));
        assert!(text.contains("URL:https://www.linkedin.com/in/priya-raman-nw\r\n"));
        assert!(text.contains("ADR;TYPE=WORK:;;;San Francisco\\, CA;;;\r\n"));
        assert!(text.contains("CATEGORIES:angels\r\n"));
        // Self and bulk are not exported; notes never are.
        assert!(!text.contains("Owen Ryder"));
        assert!(!text.contains("Digest Weekly"));
        assert!(!text.contains("secret-note-text"));
        assert!(text.lines().all(|l| l.len() <= 75));
    }

    #[test]
    fn escapes_and_folds_untrusted_text() {
        let c = indexed();
        c.execute(
            "UPDATE person SET title = ?1 WHERE id = 3",
            ["Eng;\nEND:VCARD\nBEGIN:VCARD\nFN:Evil, Inc \\ x".to_string() + &"y".repeat(100)],
        )
        .unwrap();
        let mut out = Vec::new();
        export(
            &c,
            &Config::default(),
            ExportFormat::Vcf,
            &ExportOptions::default(),
            &mut out,
            NOW,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let begins = text.split("\r\n").filter(|l| *l == "BEGIN:VCARD").count();
        assert_eq!(begins, 5, "{text}");
        assert!(text.contains(r"TITLE:Eng\;\nEND:VCARD\nBEGIN:VCARD\nFN:Evil\, Inc \\ x"));
        assert!(text.contains("\r\n "), "long lines are folded");
        assert!(text.split("\r\n").all(|l| l.len() <= 75));
    }

    #[test]
    fn do_not_share_is_excluded_by_default() {
        let c = indexed();
        c.execute("UPDATE person SET do_not_share = 1 WHERE id = 2", [])
            .unwrap();
        let mut out = Vec::new();
        let r = export(
            &c,
            &Config::default(),
            ExportFormat::Vcf,
            &ExportOptions::default(),
            &mut out,
            NOW,
        )
        .unwrap();
        assert_eq!(r.excluded_do_not_share, 1);
        assert!(!String::from_utf8(out).unwrap().contains("Priya"));
    }
}
