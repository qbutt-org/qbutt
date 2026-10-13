//! Generated Japanese prose and malformed state, without third-party text.

pub use qbutt_content::*;

#[path = "../src/estar/response.rs"]
mod response;

use serde_json::{Map, Value, json};

struct Nuxt {
    slots: Vec<Value>,
}

impl Nuxt {
    fn new() -> Self {
        Self {
            slots: vec![Value::Null],
        }
    }

    fn add(&mut self, value: Value) -> usize {
        let index = self.slots.len();
        self.slots.push(value);
        index
    }

    fn object(&mut self, fields: &[(&str, usize)]) -> usize {
        let object = fields
            .iter()
            .map(|(name, slot)| (name.to_string(), json!(slot)))
            .collect::<Map<_, _>>();
        self.add(Value::Object(object))
    }

    fn finish(mut self, key: &str, page: usize, path: &str) -> String {
        let data = self.object(&[(key, page)]);
        let wrapped_data = self.add(json!(["ShallowReactive", data]));
        let path = self.add(json!(path));
        let root = self.object(&[("data", wrapped_data), ("path", path)]);
        self.slots[0] = json!(["ShallowReactive", root]);
        serde_json::to_string(&self.slots).unwrap()
    }
}

fn document(state: String, body: &str) -> String {
    format!(
        "<html><body>{body}<script id=\"__NUXT_DATA__\" type=\"application/json\">{state}</script></body></html>"
    )
}

fn search(query: &str, ticket: &str) -> String {
    let mut state = Nuxt::new();
    let work_id = state.add(json!("42"));
    let title = state.add(json!("生成した物語"));
    let published = state.add(json!("published"));
    let count = state.add(json!("1200"));
    let caption = state.add(json!("短い紹介"));
    let work = state.object(&[
        ("workId", work_id),
        ("title", title),
        ("status", published),
        ("publishedBodyCount", count),
        ("catchphrase", caption),
    ]);
    let nodes = state.add(json!([work]));
    let total = state.add(json!(1));
    let results = state.object(&[("nodes", nodes), ("totalCount", total)]);
    let query_slot = state.add(json!(query));
    let ticket_slot = state.add(json!(ticket));
    let filter = state.object(&[("keyword", query_slot), ("ex_ticket", ticket_slot)]);
    let page = state.object(&[("filter", filter), ("novels", results)]);
    document(state.finish("novels", page, "/novels"), "")
}

fn book(canonical_id: &str) -> String {
    let mut state = Nuxt::new();
    let id = state.add(json!("42"));
    let title = state.add(json!("生成した物語"));
    let published = state.add(json!("published"));
    let yes = state.add(json!(true));
    let description = state.add(json!("生成した日本語の物語。"));
    let episodes = state.add(json!(3));
    let pages = state.add(json!(6));
    let record = state.object(&[
        ("workId", id),
        ("title", title),
        ("status", published),
        ("isNovel", yes),
        ("description", description),
        ("episodeCount", episodes),
        ("publishedPageCount", pages),
    ]);
    let novel = state.add(json!(["ModelNovel", record]));
    let page = state.object(&[("novel", novel)]);
    let body = format!("<link rel=\"canonical\" href=\"https://estar.jp/novels/{canonical_id}\">");
    document(state.finish("novel_42", page, "/novels/42"), &body)
}

fn episodes(second_start: u32) -> String {
    let mut state = Nuxt::new();
    let id = state.add(json!("42"));
    let record = state.object(&[("workId", id)]);
    let novel = state.add(json!(["ModelNovel", record]));
    let page = state.object(&[("novel", novel)]);
    let body = format!(
        "<div class=episodeList>\
         <div class=item><div class=chapterTitle>第一部</div><a class=link><div class=label>第一話</div><p class=meta>1</p></a></div>\
         <div class=item><a class=link><div class=label>第二話</div><p class=meta>{second_start}</p></a></div>\
         <div class=item><div class=chapterTitle>第二部</div><a class=link><div class=label>第三話</div><p class=meta>5</p></a></div>\
         </div>"
    );
    document(
        state.finish("novel_episode_42", page, "/novels/42/episodes?page=1"),
        &body,
    )
}

fn viewer(pay_required: bool, selected_page: u32, content: &str) -> String {
    let mut state = Nuxt::new();
    let id = state.add(json!("42"));
    let number = state.add(json!(selected_page));
    let title = state.add(json!(if selected_page == 3 { "第二話" } else { "" }));
    let episode_number = state.add(json!(2));
    let pay = state.add(json!(pay_required));
    let page_record = state.object(&[
        ("pageNo", number),
        ("title", title),
        ("episodeNo", episode_number),
        ("payRequired", pay),
    ]);
    let nodes = state.add(json!([page_record]));
    let pages = state.object(&[("nodes", nodes)]);
    let novel_record = state.object(&[("workId", id), ("pages", pages)]);
    let novel = state.add(json!(["ModelNovel", novel_record]));
    let page = state.object(&[("novel", novel)]);
    document(
        state.finish("novel_viewer", page, "/novels/42/viewer?page=3"),
        &format!("<div class=content lang=ja>{content}</div>"),
    )
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let works = response::search(&search("生成", "1"), "生成")?;
    assert_eq!(works.len(), 1);
    assert_eq!(works[0].variants[0].languages, ["ja"]);
    assert!(matches!(
        response::search(&search("生成", "0"), "生成"),
        Err(Error::InvalidResponse(_))
    ));
    assert!(matches!(
        response::book(&book("43"), &works[0].reference),
        Err(Error::VariantUnavailable)
    ));

    let book = response::book(&book("42"), &works[0].reference)?;
    let toc = response::episodes(&episodes(3), &book.work.reference, 1)?;
    let chapters = response::listing(&book, vec![toc])?;
    assert_eq!(chapters.len(), 3);
    assert_eq!(chapters[1].chapter.reference.id, "42/3");
    assert_eq!(chapters[1].start..chapters[1].end, 3..5);
    assert_eq!(chapters[1].chapter.volume.as_deref(), Some("第一部"));
    assert_eq!(chapters[2].chapter.volume.as_deref(), Some("第二部"));
    assert!(matches!(
        response::listing(
            &book,
            vec![response::episodes(&episodes(1), &book.work.reference, 1)?]
        ),
        Err(Error::InvalidResponse(_))
    ));
    assert!(matches!(
        response::episodes(&document("[]".into(), ""), &book.work.reference, 1),
        Err(Error::InvalidResponse(_))
    ));

    let source = "https://img.estar.jp/generated.png";
    let html = format!(
        "二行目\n<ruby>漢字<rt>かんじ</rt></ruby><b>強調</b><img src=\"{source}\" alt=\"図\">"
    );
    let page = response::viewer(
        &viewer(false, 3, &html),
        &book.work.reference,
        &chapters[1].chapter,
        3,
    )?;
    assert_eq!(page.images, [source]);
    let mut resolve = |value: &str| -> Result<Option<TextImageAsset>> {
        assert_eq!(value, source);
        Ok(Some(TextImageAsset {
            id: "generated-image".into(),
            media_type: "image/png".into(),
            bytes: b"\x89PNG\r\n\x1a\n".to_vec().into(),
        }))
    };
    let text = TextChapter::from_local_html(
        chapters[1].chapter.reference.clone(),
        &page.html,
        &mut resolve,
        None,
        None,
    )?;
    assert!(text.blocks.iter().flat_map(|block| &block.spans).any(|span| {
        matches!(span, TextSpan::Ruby { text, reading } if text == "漢字" && reading == "かんじ")
    }));
    assert!(
        text.blocks
            .iter()
            .flat_map(|block| &block.spans)
            .any(|span| {
                matches!(span, TextSpan::Text { text, bold: true, .. } if text == "強調")
            })
    );
    assert_eq!(text.assets.len(), 1);
    response::viewer(
        &viewer(false, 4, "続きの文章"),
        &book.work.reference,
        &chapters[1].chapter,
        4,
    )?;
    assert!(matches!(
        response::viewer(
            &viewer(true, 3, &html),
            &book.work.reference,
            &chapters[1].chapter,
            3
        ),
        Err(Error::UnsupportedAcquisition(_))
    ));
    assert!(matches!(
        response::viewer(
            &viewer(false, 4, &html),
            &book.work.reference,
            &chapters[1].chapter,
            3
        ),
        Err(Error::VariantUnavailable)
    ));
    println!("PASS Estar generated search, listing, formatting, image and access faults");
    Ok(())
}
