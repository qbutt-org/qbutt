use crate::Work;

/// Search source pages may match article bodies. Admit only a title or alias
/// containing the ordered query, including partial names and sequel suffixes.
pub fn matches(query: &str, work: &Work) -> bool {
    let query = query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect::<Vec<_>>();
    if query.is_empty() {
        return false;
    }

    std::iter::once(&work.title)
        .chain(&work.aliases)
        .any(|title| matches_title(&query, title))
}

fn matches_title(query: &[char], title: &str) -> bool {
    let title = title
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();

    for start in 0..title.len() {
        if !title[start].is_alphanumeric() || (start > 0 && title[start - 1].is_alphanumeric()) {
            continue;
        }

        let mut offset = start;
        let matched = query.iter().all(|expected| {
            while title
                .get(offset)
                .is_some_and(|character| !character.is_alphanumeric())
            {
                offset += 1;
            }

            let actual = title.get(offset);
            offset += 1;
            actual == Some(expected)
        });

        if matched
            && (!query.last().unwrap().is_numeric()
                || !title
                    .get(offset)
                    .is_some_and(|character| character.is_numeric()))
        {
            return true;
        }
    }

    false
}
