use super::*;
use serde_json::json;

#[test]
fn full_transcript_keeps_all_turns_with_the_latest_reply_and_question_first() {
    let messages = json!([
        {"info":{"id":"msg_1","role":"user","time":{"created":1}},"parts":[{"type":"text","text":"First question"}]},
        {"info":{"id":"msg_2","role":"assistant","agent":"tracer","time":{"created":2}},"parts":[{"type":"text","text":"First answer"}]},
        {"info":{"id":"msg_3","role":"user","time":{"created":3}},"parts":[{"type":"text","text":"Latest question"},{"type":"file","filename":"image.png"}]},
        {"info":{"id":"msg_4","role":"assistant","agent":"tracer","time":{"created":4}},"parts":[{"type":"tool","tool":"bash","state":{"status":"completed"}},{"type":"text","text":"Latest answer\n\n```rust\nlet answer = 42;\n```"}]}
    ]);
    let text = transcript(serde_json::from_value(messages).unwrap());
    let positions = [
        "Latest answer",
        "Latest question",
        "First answer",
        "First question",
    ]
    .map(|part| text.find(part).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(text.contains("## You"));
    assert!(text.contains("## Agent · tracer"));
    assert!(text.contains("Attachment: image.png"));
    assert!(text.contains("Tool: bash (completed)"));
    assert!(text.contains("```rust\nlet answer = 42;\n```"));
    assert_eq!(transcript(vec![]), "No messages yet.");
}

#[test]
fn latest_turn_keeps_the_newest_question_and_its_completion_timing() {
    let messages = json!([
        {"info":{"id":"msg_1","role":"user","time":{"created":1}},"parts":[{"type":"text","text":"First question"}]},
        {"info":{"id":"msg_2","role":"user","time":{"created":3}},"parts":[{"type":"text","text":"Latest\nquestion"},{"type":"file","filename":"image.png"}]},
        {"info":{"id":"msg_3","role":"assistant","parentID":"msg_2","time":{"created":4,"completed":9}},"parts":[{"type":"text","text":"Latest answer"}]}
    ]);
    assert_eq!(
        latest_turn(serde_json::from_value(messages).unwrap()),
        Some(LatestTurn {
            question: Some("Latest question".into()),
            started_at: 3,
            completed_at: Some(9),
        })
    );
}
