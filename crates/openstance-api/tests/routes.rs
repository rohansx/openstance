//! P4-API-03: every route of interfaces.md section 3.1 returns the shared
//! JSON shape: the same document the MCP tool (and so the CLI `--json`)
//! returns for the same input and `now`.

mod common;

use axum::body::Body;
use axum::http::{Method, StatusCode};
use common::*;
use openstance_core::resolve::lookup::PersonKey;
use serde_json::{json, Value};

/// The MCP tool's structured output for `args` on a reader.
async fn tool(h: &Harness, name: &'static str, args: Value) -> Value {
    h.core(move |g| {
        let out = openstance_mcp::tools::call(g, name, args.as_object().cloned()).unwrap();
        out.structured
    })
    .await
}

#[tokio::test]
async fn get_people_equals_search_people() {
    let h = harness();
    let (status, v) = h.get("/people?q=northwind&limit=5").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let want = tool(
        &h,
        "search_people",
        json!({ "text": "northwind", "limit": 5 }),
    )
    .await;
    assert_eq!(v["results"][0]["person"]["display_name"], "Priya Raman");
    assert_eq!(without_timing(v), without_timing(want));
}

/// `took_ms` differs between two runs; everything else must not.
fn without_timing(mut v: Value) -> Value {
    v["took_ms"] = Value::Null;
    v
}

#[tokio::test]
async fn get_people_maps_every_filter() {
    let h = harness();
    let uri = "/people?q=founders&company=Northwind&company=Contoso&title=ceo&since=2025&met=true&replied=true&min_score=0.25&include_bulk=false&limit=3&parse=false";
    let (status, mut v) = h.get(uri).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let want = tool(
        &h,
        "search_people",
        json!({ "text": "founders", "company": ["Northwind", "Contoso"], "title": ["ceo"],
                "since": "2025", "met_in_person": true, "replied": true, "min_score": 0.25,
                "include_bulk": false, "limit": 3, "parse": false }),
    )
    .await;
    v["took_ms"] = Value::Null;
    assert_eq!(v, without_timing(want));
    assert_eq!(v["query"]["company"], json!(["Northwind", "Contoso"]));
    assert_eq!(v["query"]["met_in_person"], true);
}

#[tokio::test]
async fn get_people_rejects_bad_parameters() {
    let h = harness();
    for uri in [
        "/people?nope=1",
        "/people?met=maybe",
        "/people?limit=0",
        "/people?limit=abc",
        "/people?since=someday",
        "/people?min_score=2",
    ] {
        let (status, v) = h.get(uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {v}");
        assert_eq!(v["error"]["code"], "bad_request");
    }
}

#[tokio::test]
async fn post_search_takes_the_mcp_input() {
    let h = harness();
    let body = json!({ "text": "devtools dinner", "limit": 4 });
    let (status, v) = h.call(Method::POST, "/search", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let want = tool(&h, "search_people", body).await;
    assert_eq!(without_timing(v), without_timing(want));
    let (status, _) = h
        .call(Method::POST, "/search", Some(json!({ "bogus": 1 })))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_person_equals_get_person_tool() {
    let h = harness();
    let (status, v) = h.get("/people/2?interactions=2&mutuals=1").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let want = tool(
        &h,
        "get_person",
        json!({ "person": "2", "interactions": 2, "mutuals": 1 }),
    )
    .await;
    assert_eq!(v, want);
    // Any reference works in the path, URL-encoded.
    let (status, v2) = h.get("/people/priya%40northwind.example").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v2["person"]["id"], 2);
    let (status, v) = h.get("/people/999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");
}

#[tokio::test]
async fn ties_between_two_people() {
    let h = harness();
    let (status, v) = h.get("/people/2/ties/3").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let want = h
        .core(|g| {
            serde_json::to_value(
                g.tie_evidence(&PersonKey::Id(2), &PersonKey::Id(3), 10)
                    .unwrap(),
            )
            .unwrap()
        })
        .await;
    assert_eq!(v, want);
    assert_eq!(v["interactions"][0]["subject"], "Board deck draft");
}

#[tokio::test]
async fn connections_equals_how_connected() {
    let h = harness();
    let (status, v) = h.get("/connections?target=3&limit=2").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let want = tool(&h, "how_connected", json!({ "target": "3", "limit": 2 })).await;
    assert_eq!(v, want);
    let (status, _) = h.get("/connections").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn playbook_path_with_default_egress_returns_the_quote_only() {
    let h = harness_with(
        openstance_core::Config::default(),
        openstance_core::Providers {
            enricher: Box::new(openstance_enrich::mock::MockEnricher::default()),
            ..openstance_core::Providers::null()
        },
    );
    let (status, v) = h
        .call(
            Method::POST,
            "/playbooks/path",
            Some(json!({ "input": { "target": "Kai Example", "company": "Fabrikam" } })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["playbook"], "path");
    assert_eq!(v["status"], "quote");
    assert_eq!(v["receipt"]["requests_sent"], 0);
    let n: i64 = h
        .core(|g| {
            g.conn()
                .query_row(
                    "SELECT count(*) FROM enrich_log WHERE status NOT IN ('quoted', 'blocked')",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        })
        .await;
    assert_eq!(n, 0, "nothing was sent");
}

#[tokio::test]
async fn playbook_errors() {
    let h = harness();
    let (status, v) = h
        .call(
            Method::POST,
            "/playbooks/nope",
            Some(json!({ "input": {} })),
        )
        .await;
    assert!(status.is_client_error(), "{status} {v}");
    let (status, _) = h
        .call(
            Method::POST,
            "/playbooks/path",
            Some(json!({ "input": { "target": "x" }, "max_usd": -1 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn quote_equals_quote_enrich() {
    let h = harness();
    let body = json!({ "requests": [ { "kind": "profile", "name": "Kai Example" } ] });
    let (status, v) = h.call(Method::POST, "/quote", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let want = h
        .pool
        .write(move |g| {
            Ok(
                openstance_mcp::tools::call(g, "quote_enrich", body.as_object().cloned())
                    .unwrap()
                    .structured,
            )
        })
        .await
        .unwrap();
    let strip = |mut v: Value| {
        v["quoted_at"] = Value::Null;
        v
    };
    assert_eq!(strip(v), strip(want));
}

#[tokio::test]
async fn edits_apply_and_persist() {
    let h = harness();
    let (status, v) = h
        .call(
            Method::POST,
            "/edits",
            Some(json!({ "ops": [
                { "op": "set", "person": 2, "field": "title", "value": "Chief Brewer" },
                { "op": "tag", "person": 2, "value": "brewing" }
            ]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["applied"], 2);
    assert_eq!(v["people"][0]["id"], 2);
    assert_eq!(v["people"][0]["title"], "Chief Brewer");
    let (_, p) = h.get("/people/2").await;
    assert_eq!(p["person"]["title"], "Chief Brewer");
    assert_eq!(p["person"]["field_sources"]["title"], "override");

    let (status, v) = h
        .call(
            Method::POST,
            "/edits",
            Some(json!({ "ops": [ { "op": "set", "person": 2, "field": "shoe", "value": "x" } ] })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let (status, _) = h
        .call(Method::POST, "/edits", Some(json!({ "ops": [ { "op": "set", "person": 999, "field": "title", "value": "x" } ] })))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn review_equals_the_core_queue() {
    let h = harness();
    let (status, v) = h.get("/review").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let want = h
        .core(|g| serde_json::to_value(g.review().unwrap()).unwrap())
        .await;
    assert_eq!(v, want);
    let (status, _) = h.get("/review?min_confidence=2").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn views_create_list_search_and_export() {
    let h = harness();
    let (status, v) = h
        .call(
            Method::POST,
            "/views",
            Some(json!({ "name": "nw", "query": "northwind", "ttl": "30d", "to": ["self"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["name"], "nw");
    assert_eq!(v["direction"], "own");

    let (status, list) = h.get("/views").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["views"][0]["name"], "nw");
    let (status, _) = h.get("/views?direction=sideways").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, headers, bytes) = h
        .send(
            req(Method::POST, "/views/nw/export")
                .header("host", HOST)
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/octet-stream");
    assert!(headers["content-disposition"]
        .to_str()
        .unwrap()
        .contains("nw.ost"));
    assert!(bytes.len() > 100);

    // Import the file back (it is addressed to my own key) and search it.
    let boundary = "XBOUNDARYX";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"nw.ost\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let (status, _, out) = h
        .send(
            req(Method::POST, "/views/import")
                .header("host", HOST)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    let imported: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(status, StatusCode::OK, "{imported}");
    assert_eq!(imported["view"]["name"], "nw");
    let (status, found) = h.get("/views/*/search?q=priya").await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["v"], 1);
    assert!(found["results"].as_array().is_some());
}

#[tokio::test]
async fn import_streams_files_through_the_importer() {
    let opts = openstance_core::import::ImportOptions::default();
    let h = harness_with(
        openstance_core::Config::default(),
        openstance_core::Providers::with_importers(openstance_core::import::registry(&opts)),
    );
    let vcf = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Quinn Example\r\nEMAIL:quinn@fabrikam.example\r\nORG:Fabrikam\r\nEND:VCARD\r\n";
    let boundary = "B0UNDARY";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"label\"\r\n\r\nphone contacts\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"../../contacts.vcf\"\r\nContent-Type: text/vcard\r\n\r\n{vcf}\r\n--{boundary}--\r\n"
    );
    let (status, _, out) = h
        .send(
            req(Method::POST, "/import")
                .header("host", HOST)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["sources"][0]["kind"], "vcf");
    assert_eq!(v["sources"][0]["label"], "phone contacts");
    assert_eq!(
        v["sources"][0]["path"], "contacts.vcf",
        "file name only, no temp path"
    );
    let (_, s) = h.get("/sources").await;
    let last = s["sources"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["path"], "contacts.vcf");
    let (_, found) = h.get("/people?q=quinn").await;
    assert_eq!(
        found["results"][0]["person"]["display_name"],
        "Quinn Example"
    );

    let (status, v) = h
        .send(
            req(Method::POST, "/import")
                .header("host", HOST)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(format!("--{boundary}--\r\n")))
                .unwrap(),
        )
        .await
        .into_status_json();
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
}

trait IntoStatusJson {
    fn into_status_json(self) -> (StatusCode, Value);
}

impl IntoStatusJson for (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    fn into_status_json(self) -> (StatusCode, Value) {
        (
            self.0,
            serde_json::from_slice(&self.2).unwrap_or(Value::Null),
        )
    }
}

#[tokio::test]
async fn sources_enrich_log_config_and_stats() {
    let h = harness();
    let (status, v) = h.get("/sources").await;
    assert_eq!(status, StatusCode::OK);
    let want = h
        .core(|g| serde_json::to_value(g.sources().unwrap()).unwrap())
        .await;
    assert_eq!(v, want);

    let (status, v) = h.get("/enrich-log?since=7d").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["totals"]["requests_sent"], 0);
    let (status, _) = h.get("/enrich-log?since=whenever").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = h.get("/enrich-log?person=2").await;
    assert_eq!(status, StatusCode::OK);

    let (status, v) = h.get("/config").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v, h.opts.config_doc);

    let (status, v) = h.get("/stats").await;
    assert_eq!(status, StatusCode::OK);
    let want = h
        .core(|g| serde_json::to_value(g.dashboard_stats().unwrap()).unwrap())
        .await;
    assert_eq!(v, want);
    assert_eq!(v["people"], 5);
}

#[tokio::test]
async fn shortlist_round_trip() {
    let h = harness();
    let (status, v) = h
        .call(
            Method::POST,
            "/shortlist",
            Some(json!({ "person": "priya@northwind.example", "note": "seed" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["entry"]["person_id"], 2);
    assert_eq!(v["entry"]["note"], "seed");
    let (status, v) = h
        .call(Method::PATCH, "/shortlist/2", Some(json!({ "note": null })))
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(v["entry"]["note"].is_null());
    let (_, list) = h.get("/shortlist").await;
    assert_eq!(list["shortlist"].as_array().unwrap().len(), 1);
    let (status, v) = h.call(Method::DELETE, "/shortlist/2", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v, json!({ "v": 1, "removed": true }));
    let (status, _) = h.call(Method::DELETE, "/shortlist/2", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = h
        .call(
            Method::POST,
            "/shortlist",
            Some(json!({ "person": 2, "extra": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn intros_round_trip() {
    let h = harness();
    let (status, v) = h
        .call(
            Method::POST,
            "/intros",
            Some(json!({ "person": "in/lena-park", "connector": 2, "follow_up": "2026-11-02" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let id = v["intro"]["id"].as_i64().unwrap();
    assert_eq!(v["intro"]["stage"], "draft");
    assert_eq!(v["intro"]["connector"]["id"], 2);
    let (status, v) = h
        .call(
            Method::PATCH,
            &format!("/intros/{id}"),
            Some(json!({ "stage": "intro_made", "connector": null, "note": "thanks Priya" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["intro"]["stage"], "intro_made");
    assert!(v["intro"]["connector"].is_null());
    assert_eq!(v["intro"]["follow_up"], "2026-11-02", "untouched");
    let (_, list) = h.get("/intros?stage=intro_made").await;
    assert_eq!(list["intros"].as_array().unwrap().len(), 1);
    let (status, _) = h.get("/intros?stage=someday").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = h
        .call(
            Method::PATCH,
            &format!("/intros/{id}"),
            Some(json!({ "stage": "teleported" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, v) = h.call(Method::DELETE, &format!("/intros/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["removed"], true);
    let (status, _) = h.call(Method::DELETE, &format!("/intros/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn views_create_accepts_a_search_request_query_and_replace() {
    let h = harness();
    let body = json!({ "name": "nw2", "ttl": "30d", "to": ["self"], "replace": true,
        "query": { "text": "northwind", "met_in_person": true, "since": "2025" } });
    let (status, v) = h.call(Method::POST, "/views", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["query"]["met_in_person"], true);
    assert_eq!(v["query"]["since"], 1_735_689_600);
    assert_eq!(
        v["query"]["company"],
        json!(["northwind"]),
        "text parsed like the CLI"
    );
    // `replace` (alias of `yes`) lets the same name be saved again.
    let (status, v) = h.call(Method::POST, "/views", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    // A core Query (as GET /views returns it) works too.
    let q = v["query"].clone();
    let (status, v) = h
        .call(
            Method::POST,
            "/views",
            Some(json!({ "name": "nw3", "query": q, "to": ["self"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["query"]["since"], 1_735_689_600);
}
