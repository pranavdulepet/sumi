use crate::{local, plan, sources, store::*};
use chrono::{Datelike, Days, NaiveDate};
use serde_json::{json, Value};
use std::{collections::HashSet, io::Read, time::Duration};

pub fn outing(q: &str, last: &Value) -> bool {
    let q = q.to_lowercase();
    (q.contains("weekend")
        && [
            "what should",
            "what can",
            "what could",
            "things to do",
            "what's on",
            "what’s on",
            "where should",
            "plans for",
            "plan my",
            "what to do",
            "any ideas",
            "what is happening",
        ]
        .iter()
        .any(|cue| q.contains(cue)))
        || (last["web"]["status"] == "place" && q.len() < 100)
}
fn city(text: &str) -> Option<&str> {
    let place = text.trim().trim_end_matches(['?', '.', '!']);
    let text = place
        .strip_suffix(" this weekend")
        .or_else(|| place.strip_suffix(" for the weekend"))
        .unwrap_or(place);
    let city = text.trim().trim_end_matches(['?', '.', '!']).trim();
    (!city.is_empty()
        && city.len() <= 55
        && city.split_whitespace().count() <= 5
        && city
            .chars()
            .all(|c| c.is_alphabetic() || " ,’-'-".contains(c)))
    .then_some(city)
}
fn weekend(day: NaiveDate) -> (NaiveDate, NaiveDate) {
    let dow = day.weekday().num_days_from_monday();
    let sat = if dow == 6 {
        day.pred_opt().unwrap()
    } else {
        day + Days::new(u64::from(5 - dow))
    };
    (sat, sat.succ_opt().unwrap())
}
pub fn prepare(
    q: &str,
    last: &Value,
    routing: &Value,
    notes: &[Value],
    day: NaiveDate,
    restricted: bool,
) -> Option<Value> {
    let lower = q.to_lowercase();
    let is_outing = outing(q, last);
    let general = routing["web"].as_str().unwrap_or("").trim();
    let explicit = [
        "search the web",
        "look up ",
        "search online",
        "latest ",
        "current price",
        "what's happening",
        "what’s happening",
        "in the news",
    ]
    .iter()
    .any(|s| lower.contains(s));
    if !is_outing
        && !explicit
        && (general.len() < 4
            || general.eq_ignore_ascii_case("none")
            || general.eq_ignore_ascii_case("null")
            || !q.contains('?'))
    {
        return None;
    }
    if !is_outing
        && !explicit
        && (lower.contains("tomorrow")
            || lower.starts_with("what did i")
            || lower.contains("split")
            || lower.contains("my "))
    {
        return None;
    }
    if restricted {
        return Some(json!({"status":"private"}));
    }
    let mut dates = Vec::new();
    let query = if is_outing {
        let supplied = lower
            .rfind(" in ")
            .and_then(|i| q.get(i + 4..))
            .and_then(city)
            .or_else(|| {
                if last["web"]["status"] == "place" {
                    city(q)
                } else {
                    None
                }
            });
        let home = notes
            .iter()
            .find(|n| n["topic"] == "home" && n["restricted"] != true);
        let Some(place) =
            supplied.or_else(|| home.and_then(|n| n["value"].as_str()).and_then(city))
        else {
            return Some(json!({"status":"place"}));
        };
        let (start, end) = weekend(day);
        dates.extend([start, end]);
        // Only broad public categories can be drawn from personal notes. Ranking happens afterwards.
        let prefs = notes
            .iter()
            .filter(|n| n["restricted"] != true && n["topic"] != "home")
            .filter_map(|n| n["text"].as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let category = if prefs.contains("printmaking") || prefs.contains("block printing") {
            "art workshops, printmaking and block printing"
        } else if [
            "art",
            "printmaking",
            "gallery",
            "galleries",
            "museum",
            "pottery",
        ]
        .iter()
        .any(|w| prefs.contains(w))
        {
            "museum exhibitions and art workshops"
        } else if ["hik", "outdoors", "nature", "walk"]
            .iter()
            .any(|w| prefs.contains(w))
        {
            "parks and guided nature walks"
        } else if ["music", "concert", "jazz"]
            .iter()
            .any(|w| prefs.contains(w))
        {
            "live music and concerts"
        } else {
            "weekend events and things to do"
        };
        format!(
            "{place} {category}, {} through {}. Official venue listings",
            start.format("%B %-d, %Y"),
            end.format("%B %-d, %Y")
        )
    } else {
        let query = if general.len() >= 4 && !general.eq_ignore_ascii_case("none") {
            general
        } else {
            q
        };
        query
            .split_whitespace()
            .take(28)
            .collect::<Vec<_>>()
            .join(" ")
    };
    if query.len() > 360 || local::sensitive(&query) || sources::audit(&query).is_none() {
        return Some(json!({"status":"private"}));
    }
    Some(json!({"status":"ready","query":query,"outing":is_outing,"dates":dates}))
}
pub fn introduction(web: &Value) -> &'static str {
    match web["status"].as_str() {
        Some("place")=>"Which city or town should I look around?",
        Some("private")=>"This conversation stays on your Mac. I can brainstorm here, or you can ask for current listings on a new sheet.",
        _ if web["outing"]==true=>"I’ll check the venue listings for a plan that fits.",
        _=>"I can look that up and bring back the source.",
    }
}
pub fn public_url(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|u| {
        matches!(u.scheme(), "https" | "http")
            && u.has_host()
            && u.username().is_empty()
            && u.password().is_none()
    })
}
fn parse(raw: &str) -> Res<Vec<Value>> {
    let mut sources = vec![];
    let mut seen = HashSet::new();
    for line in raw
        .lines()
        .filter_map(|l| l.strip_prefix("data:").map(str::trim))
        .chain(raw.trim_start().starts_with('{').then_some(raw))
    {
        let res: Value = serde_json::from_str(line)
            .map_err(|_| "The search service returned an unreadable response.")?;
        if res["error"].is_object() || res["result"]["isError"] == true {
            return Err("Web search is unavailable. Try again in a moment.".into());
        }
        for item in res["result"]["content"].as_array().into_iter().flatten() {
            let Some(text) = item["text"].as_str() else {
                continue;
            };
            for chunk in text.split("\n---\n") {
                let mut lines = chunk.trim().lines();
                let Some(title) = lines.next().and_then(|l| l.strip_prefix("Title: ")) else {
                    continue;
                };
                let Some(url) = lines
                    .next()
                    .and_then(|l| l.strip_prefix("URL: "))
                    .map(str::trim)
                else {
                    continue;
                };
                if !public_url(url) || !seen.insert(url.to_owned()) || title.trim().is_empty() {
                    continue;
                }
                let content = lines
                    .skip_while(|l| !matches!(l.trim(), "Highlights:" | "Text:" | "Content:"))
                    .skip(1)
                    .filter(|l| !l.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                sources.push(json!({"title":title.trim().chars().take(180).collect::<String>(),"url":url,"excerpt":content.chars().take(2600).collect::<String>()}));
                if sources.len() == 5 {
                    return Ok(sources);
                }
            }
        }
    }
    Ok(sources)
}
fn search(query: &str) -> Res<Vec<Value>> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(22))
        .build();
    let response=agent.post("https://mcp.exa.ai/mcp").set("Accept","application/json, text/event-stream").send_json(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"web_search_exa","arguments":{"query":query,"numResults":5,"objective":"Find primary sources that directly answer this query. For events prioritize official venue pages that explicitly list the requested dates, activities and visiting details."}}})).map_err(|_|"I couldn’t reach web search. Check your connection and try again.".to_owned())?;
    let mut body = String::new();
    response
        .into_reader()
        .take(512_001)
        .read_to_string(&mut body)
        .msg()?;
    if body.len() > 512_000 {
        return Err("The search response was too large. Try a narrower question.".into());
    }
    parse(&body)
}

fn short_quote(line: &str) -> String {
    let words: Vec<_> = line.split_whitespace().collect();
    if words.len() > 22 {
        format!("{}…", words[..22].join(" "))
    } else {
        words.join(" ")
    }
}
fn display_lines(page: &Value) -> Vec<String> {
    page["excerpt"]
        .as_str()
        .unwrap_or("")
        .lines()
        .map(|s| s.trim_matches(|c: char| c.is_whitespace() || c == '#' || c == '*'))
        .filter(|s| s.len() > 4 && !s.contains("://") && !s.starts_with("Photo by"))
        .take(36)
        .map(|s| s.chars().take(240).collect())
        .collect()
}
fn mentions_day(line: &str, d: NaiveDate) -> bool {
    let s = line.to_lowercase();
    let tokens: Vec<_> = s
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.iter().any(|w| {
        w.len() == 4 && w.starts_with("20") && w.parse::<i32>().is_ok_and(|y| y != d.year())
    }) {
        return false;
    }
    ["%B %-d", "%b %-d", "%Y-%m-%d", "%-m/%-d/%Y", "%m/%d/%Y"]
        .iter()
        .any(|f| {
            let date = d.format(f).to_string().to_lowercase();
            s.match_indices(&date).any(|(i, _)| {
                s[i + date.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_ascii_digit())
                    && (i == 0 || !s[..i].chars().last().is_some_and(|c| c.is_alphanumeric()))
            })
        })
}

// When a calendar page contains several events, keep each activity under its
// own date heading. Separate matches elsewhere on a page cannot verify a date.
fn dated_choice(
    pages: &[Value],
    dates: &[NaiveDate],
    prefs: &[&Value],
) -> Option<(usize, String, String, Option<usize>)> {
    use std::sync::LazyLock;
    static DATE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)\b(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\.? \d{1,2}\b|\b\d{1,2}/\d{1,2}(?:/\d{2,4})?\b|\b20\d{2}-\d{2}-\d{2}\b").unwrap()
    });
    let mut best = None;
    let mut best_score = 0;
    for (i, p) in pages.iter().enumerate() {
        let lines = display_lines(p);
        let mut active: Option<(usize, &str)> = None;
        for (j, line) in lines.iter().enumerate() {
            let dated = dates.iter().any(|d| mentions_day(line, *d));
            if dated {
                active = Some((j, line));
            } else if DATE.is_match(line) {
                active = None;
            }
            let Some((start, heading)) = active else {
                continue;
            };
            if j - start > 4 {
                continue;
            }
            let lower = line.to_lowercase();
            let topic = [
                "workshop",
                "printing",
                "sculpt",
                "concert",
                "guided",
                "walk",
                "hike",
                "drop-in",
                "exhibition",
                "gallery",
                "jazz",
                "festival",
                "tour",
                "printmaking",
                "art making",
            ]
            .iter()
            .any(|term| lower.contains(term));
            if !topic {
                continue;
            }
            let mut matched = None;
            let mut best_overlap = 0;
            for (k, n) in prefs.iter().enumerate() {
                let cue = n["text"].as_str().unwrap_or("").to_lowercase();
                let terms = crate::conversation::words(&cue);
                let print = cue.contains("printmaking")
                    && (lower.contains("printmaking") || lower.contains("printing"));
                let overlap = crate::conversation::words(line)
                    .intersection(&terms)
                    .count()
                    + if print { 3 } else { 0 };
                if overlap > best_overlap {
                    matched = Some(k);
                    best_overlap = overlap;
                }
            }
            let score = 6 * best_overlap + if dated { 4 } else { 1 };
            if score > best_score {
                best_score = score;
                best = Some((i, line.to_owned(), heading.to_owned(), matched));
            }
        }
    }
    best
}

pub fn complete(s: &Store, turn: &mut Value, model: &Value, notes: &[Value]) -> Res<()> {
    let query = turn["web"]["query"]
        .as_str()
        .ok_or("Missing search query.")?
        .to_owned();
    let pages = search(&query)?;
    if pages.is_empty() {
        turn["reply"] =
            json!("I couldn’t find a source for that search. Try a broader topic or another town.");
        turn["web"]["status"] = json!("empty");
        return Ok(());
    }
    let dates: Vec<NaiveDate> =
        serde_json::from_value(turn["web"]["dates"].clone()).unwrap_or_default();
    let indexed: Vec<_> = pages.iter().enumerate().map(|(i,p)| {
        let lines=display_lines(p);
        json!({"number":i+1,"title":p["title"],"lines":lines.iter().enumerate().map(|(j,l)|json!([j+1,l])).collect::<Vec<_>>()})
    }).collect();
    let prefs: Vec<_> = notes.iter().filter(|n| n["topic"] != "home").collect();
    let memories: Vec<_> = prefs
        .iter()
        .enumerate()
        .map(|(i, n)| json!({"number":i+1,"fact":n["text"]}))
        .collect();
    let schema = json!({"type":"object","properties":{"source":{"type":"integer"},"line":{"type":"integer"},"answer":{"type":"string"},"memory":{"type":"integer"}},"required":["source","line","answer","memory"],"additionalProperties":false});
    let answer=plan::think(s,model,"web synthesis",include_str!("../../prompts/web.txt"),&json!({"question":turn["question"],"search":query,"sources":indexed,"preferences":memories}),220,Some(&schema)).ok().and_then(|r|serde_json::from_str::<Value>(&r).ok()).unwrap_or(Value::Null);
    fn index(value: &Value, count: usize) -> Option<usize> {
        let i = value.as_u64()?.checked_sub(1)? as usize;
        (i < count).then_some(i)
    }
    let dated = if turn["web"]["outing"] == true {
        dated_choice(&pages, &dates, &prefs)
    } else {
        None
    };
    let pick = dated
        .as_ref()
        .map(|d| d.0)
        .or_else(|| index(&answer["source"], pages.len()));
    turn["reply"] = json!("Here are the pages I found. Open a source to check the details.");
    if let Some(i) = pick {
        let title = pages[i]["title"].as_str().unwrap_or("the venue’s listing");
        let lines = display_lines(&pages[i]);
        let line = index(&answer["line"], lines.len()).and_then(|n| lines.get(n));
        if turn["web"]["outing"] == true {
            let mut reply = format!("My pick is {title}.");
            if let Some((_, activity, heading, _)) = &dated {
                reply.push_str(&format!("\n\n“{}”", short_quote(activity)));
                if heading != activity {
                    reply.push_str(&format!(
                        "\n\nListed under {}.",
                        short_quote(heading).trim_end_matches('.')
                    ));
                }
            } else {
                if let Some(detail) = line.filter(|l| !title.contains(l.as_str())) {
                    reply.push_str(&format!("\n\n“{}”", short_quote(detail)));
                }
                reply.push_str("\n\nCheck the venue for this weekend’s availability.");
            }
            turn["reply"] = json!(reply);
        } else if let Some(a) = answer["answer"]
            .as_str()
            .map(str::trim)
            .filter(|a| !a.is_empty() && !a.contains("://"))
        {
            turn["reply"] = json!(a);
        }
        let memory = if let Some((i, _, _, n)) = &dated {
            n.or_else(|| {
                if index(&answer["source"], pages.len()) == Some(*i) {
                    index(&answer["memory"], prefs.len())
                } else {
                    None
                }
            })
        } else {
            index(&answer["memory"], prefs.len())
        };
        if let Some(n) = memory {
            turn["usedMemory"] = json!(prefs[n]);
        }
    }
    turn["web"]["sources"] = json!(pages);
    turn["web"]["pick"] = json!(pick);
    turn["web"]["checkedAt"] = json!(now());
    turn["web"]["status"] = json!("done");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weekends_include_today_and_cross_year() {
        for (today, start) in [("2026-11-22", "2026-11-21"), ("2027-01-01", "2027-01-02")] {
            let (sat, sun) = weekend(today.parse().unwrap());
            assert_eq!(sat.to_string(), start);
            assert_eq!(sun, sat.succ_opt().unwrap());
        }
    }
    #[test]
    fn reads_only_public_source_links_and_surfaces_service_errors() {
        let list="Title: A\nURL: https://museum.example/events\nHighlights:\nSaturday\n---\nTitle: Bad\nURL: file:///private/data\nHighlights:\nNo";
        let raw = json!({"result":{"content":[{"text": list}]}});
        assert_eq!(
            parse(&format!("event: message\ndata: {raw}\n"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(parse(&raw.to_string()).unwrap()[0]["excerpt"], "Saturday");
        assert!(parse(r#"{"result":{"isError":true}}"#).is_err());
    }
}

#[cfg(test)]
mod privacy_tests {
    use super::*;
    #[test]
    fn asks_for_place_before_network_and_excludes_private_notes() {
        let day = "2026-11-19".parse().unwrap();
        let notes = [json!({"topic":"home","value":"Secret Town","restricted":true})];
        let q = "What should I do this weekend?";
        assert_eq!(
            prepare(q, &Value::Null, &Value::Null, &notes, day, false).unwrap()["status"],
            "place"
        );
        assert_eq!(
            prepare(q, &Value::Null, &Value::Null, &notes, day, true).unwrap()["status"],
            "private"
        );
        let last = json!({"web":{"status":"place"}});
        let public = prepare("San Francisco", &last, &Value::Null, &notes, day, false).unwrap();
        let query = public["query"].as_str().unwrap();
        assert!(query.starts_with("San Francisco"));
        assert!(!query.contains("Secret"));
        assert!(query.contains("November 21, 2026"));
        assert!(prepare(
            "What is 2 + 2?",
            &Value::Null,
            &Value::Null,
            &[],
            day,
            false
        )
        .is_none());
        assert!(!mentions_day(
            "November 21, 2024 brunch",
            day + Days::new(2)
        ));
    }
}

#[cfg(test)]
mod listing_tests {
    use super::*;
    #[test]
    fn does_not_attach_another_weekends_workshop_to_this_saturday() {
        let pages = [
            json!({"excerpt":"Saturday, November 21, 2026\nDrop-In Art Making: Expressive Block Printing\nSaturday, December 5, 2026\nSculpted Pendants workshop"}),
        ];
        let n = json!({"text":"I enjoy sculpted pendants workshops"});
        let picked = dated_choice(&pages, &["2026-11-21".parse().unwrap()], &[&n]).unwrap();
        assert!(picked.1.contains("Block Printing"));
        assert!(picked.2.contains("November 21"));
    }
}
