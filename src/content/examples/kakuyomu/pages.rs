use serde_json::{Value, json};

pub const WORK: &str = "29120516058121712910";
pub const FIRST: &str = "29120516058121712911";
pub const SECOND: &str = "29120516058121712912";

pub fn document(path: &str, case: &str) -> String {
    if path.contains("/episodes/") {
        return chapter(path, case);
    }
    let search = path.starts_with("/search");
    let mut work = json!({
        "__typename": "Work", "id": WORK, "title": "生成した物語 & 原文",
        "introduction": "検証用に生成した文章。", "publicEpisodeCount": 2,
        "kakuyomuNextWork": null,
        "tableOfContentsV2": [node("TableOfContentsChapter:arc1"), node("TableOfContentsChapter:arc2")]
    });
    if case == "next" {
        work["kakuyomuNextWork"] = node("KakuyomuNextWork:42");
    }
    if case == "missing-policy" {
        work.as_object_mut().unwrap().remove("kakuyomuNextWork");
    }
    if case == "changed-node" {
        work["id"] = json!("999");
    }
    let mut state = json!({
        format!("Work:{WORK}"): work,
        "Work:999": {
            "__typename": "Work", "id": "999", "title": "Unrelated recommendation",
            "publicEpisodeCount": 1, "kakuyomuNextWork": null
        },
        "TableOfContentsChapter:arc1": {
            "__typename": "TableOfContentsChapter", "id": "arc1",
            "chapter": node("Chapter:arc1"), "episodeUnions": [node(&format!("Episode:{FIRST}"))]
        },
        "TableOfContentsChapter:arc2": {
            "__typename": "TableOfContentsChapter", "id": "arc2",
            "chapter": node("Chapter:arc2"), "episodeUnions": [node(&format!("Episode:{SECOND}"))]
        },
        "Chapter:arc1": { "__typename": "Chapter", "id": "arc1", "title": "第一章" },
        "Chapter:arc2": { "__typename": "Chapter", "id": "arc2", "title": "第二章" },
        format!("Episode:{FIRST}"): { "__typename": "Episode", "id": FIRST, "title": "最初の話" },
        format!("Episode:{SECOND}"): { "__typename": "Episode", "id": SECOND, "title": "二つ目の話" }
    });
    if case == "duplicate-episode" {
        state["TableOfContentsChapter:arc2"]["episodeUnions"] =
            json!([node(&format!("Episode:{FIRST}"))]);
    }
    if case == "incomplete" {
        state["TableOfContentsChapter:arc2"]["episodeUnions"] = json!([]);
    }
    if case == "changed-episode" {
        state[format!("Episode:{SECOND}")]["id"] = json!("999");
    }
    if case == "huge-heading" {
        state["Chapter:arc1"]["title"] = json!("x".repeat(4097));
    }
    let query = if case == "wrong-query" {
        "other"
    } else {
        "generated"
    };
    let empty = case == "empty";
    let key = format!("searchWorks({})", json!({ "query": query, "offset": 0 }));
    state["ROOT_QUERY"] = json!({ key: {
        "__typename": "SearchWorkConnection",
        "nodes": if empty { json!([]) } else { json!([node(&format!("Work:{WORK}"))]) },
        "totalCount": if empty || case == "bad-count" { 0 } else { 1 }
    }});
    let data = json!({ "props": { "pageProps": { "__APOLLO_STATE__": state } } });
    let canonical = if case == "foreign-work" {
        "/works/999".into()
    } else {
        format!("/works/{WORK}")
    };
    let canonical = if search { "/search".into() } else { canonical };
    format!(
        "<!doctype html><link rel='canonical' href='https://kakuyomu.jp{canonical}'><script id='__NEXT_DATA__' type='application/json'>{data}</script>"
    )
}

fn node(key: &str) -> Value {
    json!({ "__ref": key })
}

fn chapter(path: &str, case: &str) -> String {
    let canonical = if case == "foreign-chapter" {
        "/works/999/episodes/999"
    } else {
        path
    };
    let identity = if case == "wrong-body" {
        "/works/999/episodes/999"
    } else {
        path
    };
    let class = if case == "missing-body" {
        "paywall"
    } else {
        "widget-episodeBody"
    };
    format!(
        "<!doctype html><link rel='canonical' href='https://kakuyomu.jp{canonical}'><nav>unsafe navigation</nav><div class='{class}' data-viewer-history-path='{identity}/history'><p>　生成した<ruby>漢字<rt>かんじ</rt></ruby>と<strong>強調</strong>。</p><p>二つ目の段落。<em>静かな声</em><script>unsafe()</script></p></div><footer>unsafe footer</footer>"
    )
}
