use crate::{conversation::words, plan, store::*};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use serde_json::{json, Value};

// This parser accepts arithmetic only. The planner cannot request an external process.
struct Expression<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Expression<'_> {
    fn next(&mut self) -> Option<u8> {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
        self.bytes.get(self.at).copied()
    }
    fn value(&mut self, precedence: u8) -> Option<f64> {
        let mut n = match self.next()? {
            b'(' => {
                self.at += 1;
                let n = self.value(0)?;
                if self.next()? != b')' {
                    return None;
                }
                self.at += 1;
                n
            }
            b'-' => {
                self.at += 1;
                -self.value(3)?
            }
            b'+' => {
                self.at += 1;
                self.value(3)?
            }
            _ => {
                let start = self.at;
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|v| v.is_ascii_digit() || *v == b'.')
                {
                    self.at += 1
                }
                std::str::from_utf8(&self.bytes[start..self.at])
                    .ok()?
                    .parse()
                    .ok()?
            }
        };
        loop {
            let Some(op) = self.next() else { break };
            if op == b'%' {
                self.at += 1;
                n /= 100.;
                continue;
            }
            let weight = match op {
                b'+' | b'-' => 1,
                b'*' | b'/' => 2,
                _ => break,
            };
            if weight < precedence {
                break;
            }
            self.at += 1;
            let rhs = self.value(weight + 1)?;
            n = match op {
                b'+' => n + rhs,
                b'-' => n - rhs,
                b'*' => n * rhs,
                _ => n / rhs,
            };
        }
        n.is_finite().then_some(n)
    }
}
fn calculate(expression: &str) -> Option<f64> {
    if expression.len() > 160 {
        return None;
    }
    let mut parser = Expression {
        bytes: expression.as_bytes(),
        at: 0,
    };
    let n = parser.value(0)?;
    (parser.next().is_none() && n.abs() <= 1e15).then_some(n)
}

// Resolve relative words against a civil date on the Mac, including across DST.
pub(crate) fn calendar(base: NaiveDate, when: &str, offset: i64) -> Option<NaiveDate> {
    let value = when.to_lowercase();
    let start = match value.trim() {
        "today" | "now" => base,
        "tomorrow" => base.succ_opt()?,
        "yesterday" => base.pred_opt()?,
        word => match NaiveDate::parse_from_str(word, "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => {
                let days = [
                    "monday",
                    "tuesday",
                    "wednesday",
                    "thursday",
                    "friday",
                    "saturday",
                    "sunday",
                ];
                let target = days
                    .iter()
                    .position(|v| *v == word.strip_prefix("next ").unwrap_or(word))?
                    as i64;
                let delta =
                    (target - i64::from(base.weekday().num_days_from_monday())).rem_euclid(7);
                base.checked_add_signed(Duration::days(if delta == 0 { 7 } else { delta }))?
            }
        },
    };
    if offset.unsigned_abs() > 36525 {
        return None;
    }
    start.checked_add_signed(Duration::days(offset))
}

fn usable_text<'a>(routing: &'a Value, key: &str) -> Option<&'a str> {
    routing[key].as_str().map(str::trim).filter(|s| {
        !s.is_empty() && !s.eq_ignore_ascii_case("none") && !s.eq_ignore_ascii_case("null")
    })
}

fn numbers(text: &str) -> Vec<f64> {
    static NUMBER: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[0-9]+(?:\.[0-9]+)?|[a-z]+").unwrap());
    let small = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    NUMBER
        .find_iter(&text.to_lowercase().replace(',', ""))
        .filter_map(|m| {
            m.as_str().parse().ok().or_else(|| {
                small
                    .iter()
                    .position(|v| *v == m.as_str())
                    .map(|v| v as f64)
            })
        })
        .collect()
}
pub fn archive_question(q: &str) -> bool {
    let lower = q.to_lowercase();
    [
        "imported chat",
        "previous chat",
        "past chat",
        "chat export",
        "old chat",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
}

pub(crate) fn asked_speaker(question: &str) -> &'static str {
    static ASSISTANT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
        r"(?i)\b(?:(?:did|had|have) (?:you|sumi|the assistant) (?:say|suggest|recommend|call|write|answer|reply|mention)|(?:you|sumi|the assistant) (?:said|suggested|recommended|wrote|replied|called|mentioned)|your (?:previous|earlier) (?:answer|reply|recommendation))\b").unwrap()
    });
    static HUMAN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
        r"(?i)\b(?:(?:did|had|have) i (?:say|ask|decide|mention|call|write|tell)|i (?:said|asked|decided|mentioned|called|wrote|told))\b").unwrap()
    });
    if ASSISTANT.is_match(question) {
        "assistant"
    } else if !HUMAN.is_match(question) && archive_question(question) {
        "both"
    } else {
        "user"
    }
}

// A match can occur well after the first paragraph of an old message.
fn passage_excerpt(text: &str, query: &std::collections::HashSet<String>) -> String {
    const LENGTH: usize = 650;
    let chars: Vec<_> = text.chars().collect();
    if chars.len() <= LENGTH {
        return text.to_owned();
    }
    let last = chars.len() - LENGTH;
    let mut best = (0, 0);
    for start in (0..last).step_by(LENGTH / 2).chain(std::iter::once(last)) {
        let window: String = chars[start..start + LENGTH].iter().collect();
        let score = words(&window).intersection(query).count();
        if score > best.0 {
            best = (score, start);
        }
    }
    let start = best.1;
    let mut excerpt: String = chars[start..start + LENGTH].iter().collect();
    if start > 0 {
        excerpt.insert(0, '…');
    }
    if start < last {
        excerpt.push('…');
    }
    excerpt
}

fn historical_question(q: &str) -> bool {
    let q = q.to_lowercase();
    archive_question(&q)
        || asked_speaker(&q) == "assistant"
        || [
            "i mentioned",
            "i told you",
            "we discussed",
            "originally",
            "used to",
            "over time",
            "how did that change",
            "how has",
            "what changed",
            "history of",
            "timeline",
            "what did i",
            "what have i",
            "last time",
            "did i say",
            "did i decide",
            "remind me",
            "in my notebook",
            "imported chat",
            "past chat",
            "chat export",
        ]
        .iter()
        .any(|v| q.contains(v))
}
pub fn wants_timeline(q: &str) -> bool {
    let q = q.to_lowercase();
    [
        "originally",
        "used to",
        "over time",
        "how did that change",
        "what changed",
        "history of",
        "timeline",
    ]
    .iter()
    .any(|v| q.contains(v))
}
fn dates_in_scope(q: &str, date: NaiveDate, today: NaiveDate) -> bool {
    let q = q.to_lowercase();
    if q.contains("last week") || q.contains("this week") {
        let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
        let start = if q.contains("last week") {
            monday - Duration::weeks(1)
        } else {
            monday
        };
        return date >= start && date < start + Duration::weeks(1);
    }
    if q.contains("yesterday") {
        return Some(date) == today.pred_opt();
    }
    if q.contains("today") {
        return date == today;
    }
    static ISO: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\b(?:as of|on|before|after|since) (\d{4}-\d{2}-\d{2})\b").unwrap()
    });
    if let Some(c) = ISO.captures(&q) {
        if let Ok(d) = NaiveDate::parse_from_str(&c[1], "%Y-%m-%d") {
            return match c[0].split_whitespace().next() {
                Some("on") => date == d,
                Some("before") => date < d,
                Some("after") => date > d,
                Some("since") => date >= d,
                _ => date <= d,
            };
        }
    }
    true
}

pub fn select(
    s: &Store,
    model: &Value,
    q: &str,
    last: &Value,
    candidates: &[Value],
    now: DateTime<Local>,
) -> Res<Value> {
    let simple = q
        .trim()
        .trim_end_matches(['?', '!', '.'])
        .to_ascii_lowercase();
    if matches!(
        simple.as_str(),
        "hi" | "hello" | "thank you" | "thanks" | "good morning"
    ) {
        return Ok(json!({"quick":true,"selector":"built-in","skill":0,"days":0}));
    }
    let expr = simple
        .strip_prefix("calculate ")
        .or_else(|| simple.strip_prefix("what is "))
        .unwrap_or(&simple);
    if expr.bytes().any(|c| b"*/+-".contains(&c)) && calculate(expr).is_some() {
        return Ok(json!({"quick":true,"selector":"built-in","math":expr,"days":0,"skill":0}));
    }
    let properties = json!({"web":{"type":"string"},"math":{"type":"string"},"date":{"type":"string"},"days":{"type":"integer"},"recall":{"type":"string"},"skill":{"type":"integer"},"quick":{"type":"boolean"}});
    let schema = json!({"type":"object","properties":properties,"required":["math","date","days","recall","skill","quick","web"],"additionalProperties":false});
    let menu: Vec<_> = candidates
        .iter()
        .enumerate()
        .map(|(i, v)| json!({"number":i+1,"name":v["title"]}))
        .collect();
    let payload = json!({"request":q,"previous_request":last["question"].as_str().unwrap_or("").chars().take(360).collect::<String>(),"available_skills":menu,"local_date":now.format("%F %A").to_string()});
    let response = plan::think(
        s,
        model,
        "tool selection",
        include_str!("../../prompts/tools.txt"),
        &payload,
        220,
        Some(&schema),
    )?;
    let mut route: Value = serde_json::from_str(response.trim())
        .map_err(|_| "Sumi could not read its tool selection. Try again.")?;
    if !route.is_object() {
        return Err("Sumi could not read its tool selection. Try again.".into());
    }
    let lower = q.to_lowercase();
    if (lower.starts_with("my ") || lower.starts_with("i prefer "))
        && !q.contains('?')
        && !lower.contains("please ")
        && !lower.contains("can you")
    {
        route["recall"] = json!("");
        route["date"] = json!("");
        route["math"] = json!("");
        route["web"] = json!("");
        return Ok(route);
    }
    let evidence = format!(
        "{} {} {}",
        q,
        last["question"].as_str().unwrap_or(""),
        last["reply"].as_str().unwrap_or("")
    );
    if usable_text(&route, "math").is_some_and(|expr| !grounded(expr, &evidence)) {
        route["math"] = json!("");
    }
    let temporal = [
        "today",
        "tomorrow",
        "yesterday",
        "date",
        "day",
        "week",
        "month",
        "year",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "when is",
        "how long until",
    ];
    if !temporal.iter().any(|v| lower.contains(v)) && !lower.chars().any(|c| c == '-') {
        route["date"] = json!("");
    }
    // Literal arithmetic and uncomplicated equal splits have unambiguous operands.
    static FORMULA: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^\s*(?:what is |what's |calculate |evaluate )?([0-9]+(?:\.[0-9]+)?(?:\s*[+*/×÷-]\s*[0-9]+(?:\.[0-9]+)?)+)\s*[?!.]?$").unwrap()
    });
    let current = numbers(q);
    if let Some(cap) = FORMULA.captures(q.trim()) {
        route["math"] = json!(cap[1].replace('×', "*").replace('÷', "/"));
    } else if current.len() == 2
        && (lower.contains("split") || lower.contains("divide"))
        && !lower.contains("tip")
        && !lower.contains("tax")
        && !lower.contains("plus")
    {
        static PEOPLE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r"(?i)\b([0-9]+|[a-z]+) (?:people|ways|friends|guests|diners|of us)\b",
            )
            .unwrap()
        });
        if let Some(n) = PEOPLE
            .captures(q)
            .and_then(|c| numbers(&c[1]).first().copied())
            .filter(|n| *n > 0.)
        {
            let total = if current[1] == n {
                current[0]
            } else {
                current[1]
            };
            route["math"] = json!(format!("{total}/{n}"));
        }
    }
    // "Tomorrow" already includes the day offset. Only apply an additional offset
    // when the person actually supplied another duration.
    let relative = ["tomorrow", "yesterday", "today"]
        .into_iter()
        .filter(|v| lower.contains(*v))
        .collect::<Vec<_>>();
    if relative.len() == 1
        && ![
            "days ",
            "weeks ",
            "months ",
            "years ",
            "day after",
            "day before",
            "week after",
            "week before",
        ]
        .iter()
        .any(|v| lower.contains(v))
        && !historical_question(q)
    {
        route["date"] = json!(relative[0]);
        route["days"] = json!(0);
    }
    if historical_question(q) && usable_text(&route, "recall").is_none() {
        route["recall"] = json!(q);
    }
    if usable_text(&route, "date").is_none() && !historical_question(q) {
        if let Some(day) = ["tomorrow", "today", "yesterday"]
            .into_iter()
            .find(|v| lower.contains(v))
        {
            route["date"] = json!(day);
            route["days"] = json!(0);
        }
    }
    Ok(route)
}

fn grounded(expr: &str, evidence: &str) -> bool {
    let amounts = numbers(evidence);
    let operands = numbers(expr);
    calculate(expr).is_some()
        && expr.bytes().any(|v| b"+-*/%".contains(&v))
        && !amounts.is_empty()
        && !operands.is_empty()
        && operands
            .iter()
            .all(|n| amounts.contains(n) || matches!(*n, 1.0 | 100.0))
}

fn recalled_rate(question: &str, facts: &[&str]) -> Option<String> {
    static UNIT: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bper ([a-z]+)").unwrap());
    let lower = question.to_lowercase();
    let unit = &UNIT.captures(&lower)?[1];
    let quantities = numbers(question);
    if quantities.len() != 1 || quantities[0] <= 0. {
        return None;
    }
    let denominator = quantities[0];
    let plural = format!("{unit}s");
    let es = format!("{unit}es");
    if !lower
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| {
            let noun = pair[1].trim_matches(|c: char| !c.is_alphabetic());
            (noun == unit || noun == plural || noun == es || (unit == "person" && noun == "people"))
                && numbers(pair[0]) == [denominator]
        })
    {
        return None;
    }
    // A recalled declaration is better evidence for a budget than numbers in a former question.
    let amounts: Vec<_> = facts
        .iter()
        .filter(|text| !text.trim_end().ends_with('?'))
        .filter_map(|text| {
            let n = numbers(text);
            (n.len() == 1).then(|| n[0])
        })
        .collect();
    if amounts.len() != 1 {
        return None;
    }
    Some(format!("{}/{denominator}", amounts[0]))
}

fn calculation(expr: &str) -> Value {
    let value = calculate(expr);
    let display = value
        .map(|n| {
            format!(
                "{expr} = {}",
                format!("{n:.8}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
            )
        })
        .unwrap_or_else(|| "Could not evaluate the expression".into());
    json!({"tool":"Calculator","input":expr,"value":value,"display":display})
}

pub fn run(
    s: &Store,
    routing: &Value,
    selector: &Value,
    private: bool,
    at: DateTime<Local>,
    question: &str,
) -> Res<Vec<Value>> {
    let mut results = Vec::new();
    let mut math = usable_text(routing, "math").map(str::to_owned);
    if let Some(query) = usable_text(routing, "recall") {
        let hits = recall(s, query, !private, at.with_timezone(&Utc), question)?;
        let lower = question.to_lowercase();
        let needs_math = [
            "how much",
            "how many",
            "split",
            "divide",
            "calculate",
            "total cost",
            "per person",
            "per class",
        ]
        .iter()
        .any(|cue| lower.contains(cue));
        if needs_math && math.is_none() && !wants_timeline(question) {
            let facts: Vec<_> = hits.iter().filter_map(|v| v["words"].as_str()).collect();
            // The planner only sees permitted material. Dates are excluded from calculator inputs.
            let evidence = format!("{question} {}", facts.join(" "));
            let schema = json!({"type":"object","properties":{"expression":{"type":"string"}},"required":["expression"],"additionalProperties":false});
            math = if let Some(rate) = recalled_rate(question, &facts) {
                Some(rate)
            } else if facts.is_empty() {
                None
            } else {
                let response=plan::think(s,selector,"calculation with memory","Write the arithmetic expression needed for this request using numbers in the person's earlier words and request. Use + - * / parentheses or %. Choose the most recent applicable figure. If a needed quantity is unknown, return an empty expression.",&json!({"earlier_words_oldest_first":facts,"request":question}),80,Some(&schema))?;
                let v: Value = serde_json::from_str(&response).msg()?;
                v["expression"]
                    .as_str()
                    .filter(|expr| grounded(expr, &evidence))
                    .map(str::to_owned)
            };
        }
        results.push(json!({"tool":"Notebook","display":format!("{} earlier passages",hits.len()),"passages":hits}));
    }
    if let Some(expr) = math {
        results.push(calculation(&expr));
    }
    if let Some(when) = usable_text(routing, "date") {
        let days = routing["days"].as_i64().unwrap_or(0);
        let day = calendar(at.date_naive(), when, days);
        results.push(json!({"tool":"Calendar","input":when,"offset":days,"date":day,"display":day.map(|d|d.format("%A, %B %-d, %Y").to_string()).unwrap_or_else(||"Could not resolve the date".into())}));
    }
    Ok(results)
}

pub fn recall(
    s: &Store,
    query: &str,
    hosted: bool,
    at: DateTime<Utc>,
    question: &str,
) -> Res<Vec<Value>> {
    let mut asked = words(question);
    asked.retain(|term| numbers(term).is_empty());
    let terms = words(query)
        .union(&asked)
        .filter(|word| numbers(word).is_empty())
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let db = s.db.lock().unwrap();
    // Older notebooks have no per-turn clock. Retain their sheet date as approximate provenance.
    let mut stmt = db.prepare("WITH turns AS (
        SELECT t.value AS turn, coalesce(json_extract(t.value,'$.at'),json_extract(p.v,'$.date')) AS stamp,
            json_extract(t.value,'$.at') IS NULL AS approximate, c.v AS chat
        FROM docs c JOIN docs p ON p.kind='sheet' AND p.id=c.id JOIN json_each(c.v,'$.turns') t
        WHERE c.kind='chat' AND (?1=0 OR (coalesce(json_extract(c.v,'$.private'),0)=0
            AND coalesce(json_extract(c.v,'$.networkClosed'),0)=0 AND coalesce(json_extract(p.v,'$.private'),0)=0)))
        SELECT body,stamp,approximate,role,origin FROM (
            SELECT json_extract(turn,'$.question') AS body, stamp, approximate, 'user' AS role, 'sheet' AS origin FROM turns WHERE ?2!='assistant'
            UNION ALL SELECT json_extract(turn,'$.reply'), stamp, approximate, 'assistant','sheet' FROM turns
                WHERE ?2!='user' AND (?1=0 OR NOT EXISTS (SELECT 1 FROM json_each(chat,'$.turns') f WHERE json_array_length(f.value,'$.files')>0 OR json_extract(f.value,'$.oneQuestion')=1))
            UNION ALL SELECT body,stamp,0,role,'import' FROM imported_messages WHERE ?1=0 AND (?2='both' OR role=?2)
        ) WHERE (?4=0 OR origin='import') AND EXISTS (SELECT 1 FROM json_each(?3) term WHERE instr(lower(body),term.value)>0)").msg()?;
    let lower = question.to_lowercase();
    let imports_only = lower.contains("imported chat") || lower.contains("chat export");
    let rows = stmt
        .query_map(
            rusqlite::params![
                hosted,
                asked_speaker(question),
                json!(terms).to_string(),
                imports_only
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .msg()?;
    let mut candidates = vec![];
    let mut frequency = std::collections::HashMap::new();
    for row in rows {
        let (text, date, approximate, role, origin) = row.msg()?;
        let Ok(stamp) = DateTime::parse_from_rfc3339(&date) else {
            continue;
        };
        let age = at.signed_duration_since(stamp);
        if age.num_milliseconds() < 0
            || !dates_in_scope(
                question,
                stamp.with_timezone(&Local).date_naive(),
                at.with_timezone(&Local).date_naive(),
            )
        {
            continue;
        }
        let shared: Vec<_> = terms.intersection(&words(&text)).cloned().collect();
        if shared.is_empty()
            || (hosted
                && (crate::local::sensitive(&text) || crate::sources::audit(&text).is_none()))
        {
            continue;
        }
        let recency = 1. / (1. + age.num_hours() as f64 / (24. * 30.));
        let important = if crate::conversation::enduring(&text) {
            1.5
        } else {
            0.
        };
        for term in &shared {
            *frequency.entry(term.clone()).or_insert(0usize) += 1;
        }
        candidates.push((
            shared,
            recency + important,
            stamp,
            json!({"role":role,"when":date,"approximate":approximate,"words":text,"origin":origin}),
        ));
    }
    // Use the person's own topic first. Planner-supplied synonyms are a fallback
    // when none of the passages use the original wording.
    if candidates
        .iter()
        .any(|(terms, _, _, _)| terms.iter().any(|term| asked.contains(term)))
    {
        candidates.retain(|(terms, _, _, _)| terms.iter().any(|term| asked.contains(term)));
    }
    let total = candidates.len() as f64;
    let mut hits: Vec<_> = candidates
        .into_iter()
        .map(|(terms, weight, stamp, passage)| {
            let relevance: f64 = terms
                .iter()
                .map(|term| {
                    (1. + (total + 1.) / (frequency[term] as f64 + 1.)).ln()
                        * if asked.contains(term) { 2. } else { 0.75 }
                })
                .sum();
            (relevance * (1. + 0.12 * weight), stamp, passage)
        })
        .collect();
    hits.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    if let Some((best, _, _)) = hits.first() {
        let floor = best * 0.45;
        hits.retain(|hit| hit.0 >= floor);
    }
    hits.truncate(5);
    hits.sort_by_key(|v| v.1);
    Ok(hits
        .into_iter()
        .map(|(_, _, mut passage)| {
            passage["words"] = json!(passage_excerpt(
                passage["words"].as_str().unwrap_or(""),
                &terms
            ));
            passage
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recall_respects_civil_date_boundaries_sheet_privacy_and_future_events() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().to_owned(), &[36; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        let today = Local::now().date_naive();
        let at = today
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
            .with_timezone(&Utc);
        for (days, private, text) in [
            (-2, false, "My dinner routine was different."),
            (-1, false, "Dinner at seven."),
            (-1, true, "Private dinner at eight."),
            (1, false, "Future dinner."),
        ] {
            let id = s.new_sheet().unwrap();
            s.put("chat",&json!({"id":id,"private":private,"turns":[{"at":(at+Duration::days(days)).to_rfc3339(),"question":text}]})).unwrap();
        }
        let hits = recall(&s, "dinner", true, at, "What did I say yesterday?").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["words"], "Dinner at seven.");
        assert_eq!(
            recall(&s, "dinner", false, at, "all dinner plans")
                .unwrap()
                .len(),
            3
        );
    }
    #[test]
    fn earlier_assistant_replies_keep_their_speaker_and_the_matching_passage() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().into(), &[17; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        let sheet = s.new_sheet().unwrap();
        let at = Utc::now() - Duration::hours(2);
        let long_reply = format!(
            "{} Try the copperplate workshop on Sunday.",
            "静かな場所です。".repeat(150)
        );
        let mut chat = json!({"id":sheet,"private":false,"turns":[{"at":at,"question":"Is copperplate a good fit?","reply":long_reply}]});
        s.put("chat", &chat).unwrap();
        let replies = recall(
            &s,
            "copperplate",
            true,
            Utc::now(),
            "What did you recommend for copperplate?",
        )
        .unwrap();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["role"], "assistant");
        assert!(replies[0]["words"]
            .as_str()
            .unwrap()
            .contains("workshop on Sunday"));
        let mine = recall(
            &s,
            "copperplate",
            false,
            Utc::now(),
            "What did I ask about copperplate?",
        )
        .unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0]["role"], "user");
        chat["turns"][0]["files"] = json!([{"name":"workshops.pdf"}]);
        s.put("chat", &chat).unwrap();
        assert!(recall(
            &s,
            "copperplate",
            true,
            Utc::now(),
            "What did you recommend?"
        )
        .unwrap()
        .is_empty());
    }
    #[test]
    fn arithmetic_and_civil_dates() {
        assert_eq!(calculate("(64 + 32) / 4"), Some(24.));
        assert_eq!(calculate("60 * (1 + 20%) / 4"), Some(18.));
        assert_eq!(calculate("1/0"), None);
        assert_eq!(calculate("2 8"), None);
        let base = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
        assert_eq!(
            calendar(base, "tomorrow", 1).unwrap().to_string(),
            "2024-03-01"
        );
        assert_eq!(
            calendar(base, "next Wednesday", 0).unwrap().to_string(),
            "2024-03-06"
        );
    }
}
