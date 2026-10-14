//! Authored model decisions driven by real proposal validation feedback.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Result, host};

pub(super) struct ProposalPeer {
    case: String,
    fixture: Option<PathBuf>,
    proposal: Option<Value>,
    accepted: bool,
}

impl ProposalPeer {
    pub(super) fn new(case: &str, native: &Path, fixture: Option<&Path>) -> Self {
        Self {
            case: std::fs::read_to_string(native.join("submission-case.txt"))
                .unwrap_or_else(|_| case.into()),
            fixture: fixture.map(Path::to_owned),
            proposal: None,
            accepted: false,
        }
    }

    pub(super) fn start(
        &mut self,
        output: &mut impl Write,
        thread: &str,
        mut proposal: Value,
    ) -> Result<()> {
        if self.case == "final-bypass" {
            return host::finish(
                output,
                thread,
                json!({"accepted":true,"reason":""}),
                Some(100),
            );
        }
        match self.case.as_str() {
            "retry-geometry" | "retry-third" | "invalid-geometry" | "submission-limit"
            | "exhaustion-bypass" => {
                proposal["regions"][0]["cleanupRects"][0]["x"] = json!(0.99);
            }
            "retry-typography" | "invalid-typography" => {
                proposal["regions"][0]["layout"]["width"] = json!(0.01);
                proposal["regions"][0]["layout"]["height"] = json!(0.01);
            }
            "retry-context" | "invalid-context" => {
                proposal["terms"] = json!([{
                    "source":"Ren", "target":"Ren", "forms":[],
                    "evidence":{"region_id":"not-on-page", "quote":"Ren"},
                }]);
            }
            _ => {}
        }
        submit(output, thread, &proposal, 0)?;
        self.proposal = Some(proposal);
        Ok(())
    }

    pub(super) fn reply(
        &mut self,
        output: &mut impl Write,
        thread: &str,
        trace: &mut impl Write,
        attempt: u64,
        request: &Value,
    ) -> Result<()> {
        let items = request["result"]["contentItems"].as_array().unwrap();
        assert!(items.iter().all(|item| item["type"] == "inputText"));
        let success = request["result"]["success"] == true;
        let feedback = items[0]["text"]
            .as_str()
            .ok_or("submission feedback missing")?;
        writeln!(
            trace,
            "{}",
            json!({
                "event":"page_submission", "attempt":attempt - 919,
                "accepted":success, "feedback":feedback,
            })
        )?;
        trace.flush()?;
        if success {
            let value: Value = serde_json::from_str(feedback)?;
            assert_eq!(value, json!({"accepted":true}));
            self.accepted = true;
            if self.case == "resubmit-accepted" {
                let mut proposal = self.proposal.as_ref().unwrap().clone();
                proposal["regions"] = json!([]);
                return submit(output, thread, &proposal, 1);
            }
        } else if self.case.starts_with("retry-")
            && (attempt == 920 || (self.case == "retry-third" && attempt == 921))
        {
            let expected = match self.case.as_str() {
                "retry-geometry" | "retry-third" => "cleanup rectangle",
                "retry-typography" => "Overflow",
                _ => "Context evidence",
            };
            if !feedback.contains(expected) {
                return host::finish(
                    output,
                    thread,
                    json!({
                        "accepted":false, "reason":"Required validation feedback is missing",
                    }),
                    Some(100),
                );
            }
            if self.case == "retry-third" && attempt == 920 {
                return submit(output, thread, self.proposal.as_ref().unwrap(), 1);
            }
            let proposal = match &self.fixture {
                Some(directory) => {
                    serde_json::from_slice(&std::fs::read(directory.join("proposal.json"))?)?
                }
                None => host::proposal("ru"),
            };
            submit(output, thread, &proposal, attempt - 919)?;
            self.proposal = Some(proposal);
            return Ok(());
        } else if !self.accepted && attempt < 922 {
            return submit(
                output,
                thread,
                self.proposal.as_ref().unwrap(),
                attempt - 919,
            );
        } else if !self.accepted && attempt == 922 && self.case == "submission-limit" {
            return submit(output, thread, &host::proposal("ru"), 3);
        }
        let accepted = self.accepted || self.case == "exhaustion-bypass";
        host::finish(
            output,
            thread,
            json!({
                "accepted": accepted && self.case != "final-rejected-after-acceptance",
                "reason": if accepted && self.case != "final-rejected-after-acceptance" {
                    String::new()
                } else if accepted {
                    "The model declined its accepted proposal".into()
                } else {
                    feedback.chars().take(500).collect::<String>()
                },
            }),
            Some(100),
        )
    }
}

fn submit(output: &mut impl Write, thread: &str, proposal: &Value, attempt: u64) -> Result<()> {
    host::send(
        output,
        json!({
            "id":920 + attempt, "method":"item/tool/call",
            "params":{
                "threadId":thread, "turnId":"fixture-turn", "namespace":null,
                "tool":"submit_page", "arguments":proposal,
            },
        }),
    )
}
