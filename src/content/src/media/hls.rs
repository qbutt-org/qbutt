//! Shared clear VOD playlist validation for playback and staged downloads.

pub(crate) const MAX_RESOURCES: usize = 10_000;

#[derive(Debug)]
pub(crate) enum Error {
    Format,
    Unsupported,
}

pub(crate) enum Line<'a> {
    Tag(&'a str),
    Resource {
        prefix: &'a str,
        uri: &'a str,
        suffix: &'a str,
    },
}

pub(crate) fn lines(body: &str) -> Result<Vec<&str>, Error> {
    let lines = body
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.first() != Some(&"#EXTM3U") {
        return Err(Error::Format);
    }
    Ok(lines)
}

pub(crate) fn media(lines: Vec<&str>) -> Result<Vec<Line<'_>>, Error> {
    if !lines.contains(&"#EXT-X-ENDLIST") {
        return Err(Error::Unsupported);
    }
    let mut result = Vec::with_capacity(lines.len());
    let mut pending_segment = false;
    let mut segments = 0;
    let mut resources = 0;
    for line in lines {
        if !line.starts_with('#') {
            if !pending_segment {
                return Err(Error::Format);
            }
            pending_segment = false;
            segments += 1;
            resources += 1;
            result.push(Line::Resource {
                prefix: "",
                uri: line,
                suffix: "",
            });
        } else if let Some(duration) = line.strip_prefix("#EXTINF:") {
            if pending_segment
                || !duration
                    .split(',')
                    .next()
                    .and_then(|value| value.parse::<f64>().ok())
                    .is_some_and(|value| value.is_finite() && value > 0.0 && value <= 3600.0)
            {
                return Err(Error::Format);
            }
            pending_segment = true;
            result.push(Line::Tag(line));
        } else if let Some(attributes) = line.strip_prefix("#EXT-X-MAP:") {
            if attributes.contains("BYTERANGE") {
                return Err(Error::Unsupported);
            }
            let uri = attribute(attributes, "URI").ok_or(Error::Format)?;
            let start = line.find("URI=\"").ok_or(Error::Format)? + 5;
            let end = start + uri.len();
            if line.as_bytes().get(end) != Some(&b'"') || line[end + 1..].contains("URI=") {
                return Err(Error::Format);
            }
            resources += 1;
            result.push(Line::Resource {
                prefix: &line[..start],
                uri,
                suffix: &line[end..],
            });
        } else if let Some(attributes) = line.strip_prefix("#EXT-X-KEY:") {
            if attributes != "METHOD=NONE" {
                return Err(Error::Unsupported);
            }
            result.push(Line::Tag(line));
        } else {
            let tag = line.split(':').next().unwrap_or(line);
            if tag.starts_with("#EXT-X-")
                && !matches!(
                    tag,
                    "#EXT-X-VERSION"
                        | "#EXT-X-TARGETDURATION"
                        | "#EXT-X-MEDIA-SEQUENCE"
                        | "#EXT-X-DISCONTINUITY-SEQUENCE"
                        | "#EXT-X-DISCONTINUITY"
                        | "#EXT-X-INDEPENDENT-SEGMENTS"
                        | "#EXT-X-ALLOW-CACHE"
                        | "#EXT-X-PLAYLIST-TYPE"
                        | "#EXT-X-PROGRAM-DATE-TIME"
                        | "#EXT-X-ENDLIST"
                )
                || line.contains("URI=")
                || line.starts_with("#EXT-X-PLAYLIST-TYPE:") && line != "#EXT-X-PLAYLIST-TYPE:VOD"
            {
                return Err(Error::Unsupported);
            }
            result.push(Line::Tag(line));
        }
        if resources > MAX_RESOURCES {
            return Err(Error::Unsupported);
        }
    }
    if pending_segment || segments == 0 {
        return Err(Error::Format);
    }
    Ok(result)
}

pub(crate) fn attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let mut quoted = false;
    let mut start = 0;
    for (index, byte) in attributes.bytes().enumerate() {
        if byte == b'"' {
            quoted = !quoted;
        }
        if byte != b',' || quoted {
            continue;
        }
        let field = &attributes[start..index];
        let (key, value) = field.split_once('=')?;
        if key == name {
            return Some(value.trim_matches('"'));
        }
        start = index + 1;
    }
    let (key, value) = attributes[start..].split_once('=')?;
    (key == name).then(|| value.trim_matches('"'))
}
