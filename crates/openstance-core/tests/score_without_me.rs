//! Reciprocity when `[me] identities` is empty (the default `openstance init`
//! without `--me`, and the UI/e2e setup). Gmail Takeout labels are then the
//! only direction signal: `Sent` means out, any other label means in. Before
//! the fix, inbound mail kept `direction = 'none'` and every person showed
//! "0 replies / N sent" although they plainly answered.

use std::path::PathBuf;

use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::{score, Config, FixedClock, Graph, Providers};

const NOW: i64 = 1_791_000_000; // 2026-10-03

fn write_mbox(dir: &std::path::Path) -> PathBuf {
    // I start a thread, she replies; she starts one, I answer.
    let mbox = "\
From you@example.com Fri Oct 09 09:00:00 2026
X-GM-THRID: 900
X-Gmail-Labels: Sent
Message-ID: <a.0@example.com>
Date: Fri, 09 Oct 2026 09:00:00 +0000
From: Jordan Avery <you@example.com>
To: Maya Chen <maya@ledgerline.example>
Subject: Fintech founders breakfast, November

(body)

From maya@ledgerline.example Fri Oct 09 12:00:00 2026
X-GM-THRID: 900
X-Gmail-Labels: Inbox
Message-ID: <a.1@ledgerline.example>
Date: Fri, 09 Oct 2026 12:00:00 +0000
From: Maya Chen <maya@ledgerline.example>
To: Jordan Avery <you@example.com>
Subject: Re: Fintech founders breakfast, November
In-Reply-To: <a.0@example.com>

(body)

From maya@ledgerline.example Sat Sep 12 08:00:00 2026
X-GM-THRID: 901
X-Gmail-Labels: Inbox,Important
Message-ID: <b.0@ledgerline.example>
Date: Sat, 12 Sep 2026 08:00:00 +0000
From: Maya Chen <maya@ledgerline.example>
To: Jordan Avery <you@example.com>
Subject: Ledgerline Q3 investor update

(body)

From you@example.com Sat Sep 12 10:00:00 2026
X-GM-THRID: 901
X-Gmail-Labels: Sent
Message-ID: <b.1@example.com>
Date: Sat, 12 Sep 2026 10:00:00 +0000
From: Jordan Avery <you@example.com>
To: Maya Chen <maya@ledgerline.example>
Subject: Re: Ledgerline Q3 investor update
In-Reply-To: <b.0@ledgerline.example>

(body)
";
    let path = dir.join("takeout.mbox");
    std::fs::write(&path, mbox).unwrap();
    path
}

#[test]
fn replies_count_without_me_identities() {
    let dir = tempfile::tempdir().unwrap();
    let file = write_mbox(dir.path());
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    let mut g = Graph::open_in_memory(Config::default(), providers, Box::new(FixedClock(NOW)))
        .unwrap();
    g.import(&[file], &ImportRequest::default()).unwrap();

    let maya: i64 = g
        .conn()
        .query_row(
            "SELECT person_id FROM identity WHERE value = 'maya@ledgerline.example'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let i = score::inputs(g.conn(), maya, NOW).unwrap();
    assert_eq!(i.n_out, 2, "two Sent messages to her");
    assert_eq!(i.n_replies, 1, "she answered the breakfast thread");
    assert_eq!(i.n_threads_started, 1, "she started the investor update");

    let none: i64 = g
        .conn()
        .query_row(
            "SELECT count(*) FROM interaction WHERE direction = 'none'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(none, 0, "Inbox-labelled mail is inbound");
}
