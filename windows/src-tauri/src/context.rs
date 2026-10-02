// Deciding what goes into each request to the model, so a long conversation or a
// big memory doesn't make every message cost more than the one before.
//
// Everything here is pure: it takes the full history / notes the app already
// holds and returns a smaller view of them for ONE request. The stored history
// is never changed, so nothing is lost on the user's side, and nothing leaves
// the machine that wasn't going to anyway — only less of it.

use serde_json::{json, Value};

/// Roughly what the model is shown of older turns, in tokens. The newest turn is
/// always sent whatever its size.
pub const HISTORY_BUDGET_TOKENS: usize = 16_000;

/// Notes sent when memory is small enough to send whole.
const NOTES_ALL_UNDER: usize = 12;
/// Characters of notes sent once memory is bigger than that (~375 tokens).
const NOTES_BUDGET_CHARS: usize = 1500;
/// Newest notes always included when selecting, whatever the question.
const NOTES_ALWAYS_RECENT: usize = 3;
/// Reminders listed to the model; the soonest matter most.
pub const REMINDERS_SHOWN: usize = 6;

/// Cheap token estimate: text at ~4 characters a token; base64 attachments at a
/// capped, much lower rate because the model sees a downscaled picture or page
/// text, not the raw bytes.
pub fn estimate_tokens(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len().div_ceil(4),
        Value::Array(items) => items.iter().map(estimate_tokens).sum(),
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("base64") {
                let len = map.get("data").and_then(Value::as_str).map_or(0, str::len);
                return (len / 100).min(6_000);
            }
            map.iter().map(|(k, v)| k.len().div_ceil(4) + estimate_tokens(v)).sum()
        }
        _ => 1,
    }
}

fn has_media(message: &Value) -> bool {
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks.iter().any(|b| matches!(b.get("type").and_then(Value::as_str), Some("image" | "document")))
        })
}

/// A turn starts at a real user prompt. A user message made of tool results is
/// the middle of a turn, so cutting only at turn starts never separates a
/// tool call from its result.
fn starts_turn(message: &Value) -> bool {
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return false;
    }
    match message.get("content") {
        Some(Value::Array(blocks)) => {
            !blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        }
        _ => true,
    }
}

/// The history to send: the newest turn always, as many earlier ones as fit the
/// budget (newest first, without gaps), and the first turn when it carries an
/// attachment — the conversation is usually *about* that file.
pub fn compact_history(history: &[Value], budget: usize) -> Vec<Value> {
    let starts: Vec<usize> = (0..history.len()).filter(|&i| starts_turn(&history[i])).collect();
    if starts.len() <= 1 {
        return history.to_vec();
    }
    let range = |n: usize| {
        let end = starts.get(n + 1).copied().unwrap_or(history.len());
        starts[n]..end
    };
    let cost = |n: usize| history[range(n)].iter().map(estimate_tokens).sum::<usize>();

    let last = starts.len() - 1;
    let mut keep_from = last;
    let mut spent = cost(last);
    while keep_from > 0 {
        let c = cost(keep_from - 1);
        if spent + c > budget {
            break;
        }
        spent += c;
        keep_from -= 1;
    }

    let mut out = Vec::new();
    if keep_from > 0 && range(0).clone().any(|i| has_media(&history[i])) {
        out.extend_from_slice(&history[range(0)]);
    }
    out.extend_from_slice(&history[starts[keep_from]..]);
    out
}

/// Which saved notes go into this request. Small memories are sent whole. Bigger
/// ones send the newest few plus the notes that share words with what the user
/// just wrote, up to a character budget — the model doesn't need to be told the
/// user's cat's name to answer a question about a spreadsheet.
pub fn select_notes(notes: &[String], query: &str) -> Vec<String> {
    let total: usize = notes.iter().map(String::len).sum();
    if notes.len() <= NOTES_ALL_UNDER && total <= NOTES_BUDGET_CHARS {
        return notes.to_vec();
    }

    let words = |text: &str| -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 4)
            .map(str::to_string)
            .collect()
    };
    let wanted = words(query);
    let score = |note: &str| -> usize {
        let have = words(note);
        wanted.iter().filter(|w| have.contains(w)).count()
    };

    let mut picked: Vec<usize> = Vec::new();
    let mut used = 0usize;
    let mut take = |i: usize, picked: &mut Vec<usize>| {
        if !picked.contains(&i) && used + notes[i].len() <= NOTES_BUDGET_CHARS {
            used += notes[i].len();
            picked.push(i);
        }
    };

    // Newest notes: the freshest context about the user.
    for i in (notes.len().saturating_sub(NOTES_ALWAYS_RECENT)..notes.len()).rev() {
        take(i, &mut picked);
    }
    // Then the ones that match the question, best match first (newer on ties).
    let mut ranked: Vec<(usize, usize)> =
        (0..notes.len()).map(|i| (score(&notes[i]), i)).filter(|(s, _)| *s > 0).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for (_, i) in ranked {
        take(i, &mut picked);
    }

    picked.sort_unstable(); // back to the order they were saved in
    picked.into_iter().map(|i| notes[i].clone()).collect()
}

/// Anthropic prompt caching: marks the end of the shared prefix so what repeats
/// from one message to the next (tools, system prompt, earlier turns) is billed
/// at a fraction. Providers that don't cache ignore the field.
pub fn cache_marker() -> Value {
    json!({ "type": "ephemeral" })
}

/// Puts a cache breakpoint on the last block of the last message.
pub fn mark_last_message(history: &mut [Value]) {
    let Some(last) = history.last_mut() else { return };
    if let Some(Value::String(text)) = last.get("content").cloned() {
        last["content"] = json!([{ "type": "text", "text": text }]);
    }
    if let Some(block) = last.get_mut("content").and_then(Value::as_array_mut).and_then(|b| b.last_mut()) {
        block["cache_control"] = cache_marker();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Value {
        json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
    }
    fn assistant(text: &str) -> Value {
        json!({ "role": "assistant", "content": [{ "type": "text", "text": text }] })
    }
    fn big(n: usize) -> String {
        "palabra ".repeat(n)
    }

    #[test]
    fn short_conversations_are_sent_whole() {
        let h = vec![user("hola"), assistant("hola"), user("y tú"), assistant("bien")];
        assert_eq!(compact_history(&h, HISTORY_BUDGET_TOKENS), h);
    }

    #[test]
    fn old_turns_are_dropped_when_over_budget_but_the_last_one_never_is() {
        let mut h = Vec::new();
        for i in 0..10 {
            h.push(user(&format!("pregunta {i} {}", big(3000)))); // ~3k tokens each
            h.push(assistant(&format!("respuesta {i}")));
        }
        let out = compact_history(&h, 10_000);
        assert!(out.len() < h.len(), "something must go");
        assert_eq!(out.last(), h.last(), "the newest message is always kept");
        assert!(out[0]["role"] == "user", "the request must still start with a user message");
        assert!(estimate_tokens(&Value::Array(out)) <= 10_500);
        // A single huge newest turn is still sent in full.
        let huge = vec![user("a"), assistant("b"), user(&big(40_000))];
        assert_eq!(compact_history(&huge, 100).last(), huge.last());
    }

    #[test]
    fn the_first_turn_is_kept_when_it_carries_the_attachment() {
        let file = json!({ "role": "user", "content": [
            { "type": "document", "source": { "type": "base64", "media_type": "application/pdf", "data": "A".repeat(100_000) } },
            { "type": "text", "text": "mira este pdf" },
        ]});
        let mut h = vec![file.clone(), assistant("visto")];
        for i in 0..8 {
            h.push(user(&format!("p{i} {}", big(3000))));
            h.push(assistant("ok"));
        }
        let out = compact_history(&h, 9_000);
        assert_eq!(out[0], file, "the PDF turn stays pinned at the start");
        assert_eq!(out.last(), h.last());
        assert!(out.len() < h.len());
    }

    #[test]
    fn tool_calls_are_never_separated_from_their_results() {
        let tool_use = json!({ "role": "assistant", "content": [{ "type": "tool_use", "id": "t1", "name": "remember", "input": {} }] });
        let tool_result = json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "Saved." }] });
        let mut h = vec![user(&big(4000)), assistant("a")];
        h.extend([user("recuerda x"), tool_use, tool_result, assistant("hecho")]);
        h.extend([user(&big(4000)), assistant("b")]);
        let out = compact_history(&h, 5_000);
        let uses = out.iter().filter(|m| m.to_string().contains("tool_use")).count();
        let results = out.iter().filter(|m| m.to_string().contains("tool_result")).count();
        assert_eq!(uses, results, "tool_use and tool_result stay together");
    }

    #[test]
    fn base64_attachments_are_not_counted_as_raw_text() {
        let v = json!({ "type": "base64", "data": "A".repeat(2_000_000) });
        assert_eq!(estimate_tokens(&v), 6_000);
    }

    #[test]
    fn small_memories_are_sent_whole_and_big_ones_selectively() {
        let few: Vec<String> = (0..5).map(|i| format!("nota {i}")).collect();
        assert_eq!(select_notes(&few, "lo que sea"), few);

        let mut many: Vec<String> = (0..60).map(|i| format!("The user likes hobby number {i} a lot")).collect();
        many[7] = "The user's dog is called Rocky".into();
        many[20] = "The user works as a nurse at the hospital".into();
        let picked = select_notes(&many, "what should I feed my dog Rocky?");
        assert!(picked.contains(&many[7]), "the relevant note is included");
        assert!(!picked.contains(&many[20]), "an unrelated old note is left out");
        assert!(picked.contains(many.last().unwrap()), "the newest notes always go");
        assert!(picked.iter().map(String::len).sum::<usize>() <= NOTES_BUDGET_CHARS);
        // Saved order is preserved.
        let positions: Vec<usize> = picked.iter().map(|p| many.iter().position(|m| m == p).unwrap()).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_cache_breakpoint_lands_on_the_last_block() {
        let mut h = vec![user("a"), json!({ "role": "user", "content": "texto plano" })];
        mark_last_message(&mut h);
        assert_eq!(h[1]["content"][0]["cache_control"]["type"], "ephemeral");
        assert!(h[0]["content"][0].get("cache_control").is_none());
    }
}
