//! Authored model decisions driven by real proposal validation feedback.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Result, host};

pub(super) struct ProposalPeer {
    case: String,
    fixture: Option<PathBuf>,
    proposal: Option<Value>,
    correction: Option<Value>,
    accepted: bool,
    preview_sets: usize,
}

impl ProposalPeer {
    pub(super) fn new(case: &str, native: &Path, fixture: Option<&Path>) -> Self {
        Self {
            case: std::fs::read_to_string(native.join("submission-case.txt"))
                .unwrap_or_else(|_| case.into()),
            fixture: fixture.map(Path::to_owned),
            proposal: None,
            correction: None,
            accepted: false,
            preview_sets: 0,
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
            "retry-schema-previews" => {
                proposal["regions"][0]["translation"][0]["bold"] = json!("invalid");
            }
            "repair-preview" => {
                self.correction = Some(proposal.clone());
                let index = proposal["regions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|region| region["translation"][0]["bold"] == true)
                    .ok_or("Authored preview fixture has no bold lettering")?;
                proposal["regions"][index]["translation"][0]["bold"] = json!(false);
                let width = proposal["regions"][index]["cleanupRects"][0]["width"]
                    .as_f64()
                    .unwrap();
                proposal["regions"][index]["cleanupRects"][0]["width"] = json!(width / 2.0);
            }
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
        let images: Vec<_> = items
            .iter()
            .filter(|item| item["type"] == "inputImage")
            .collect();
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
                "previewImages": images.len(),
            })
        )?;
        trace.flush()?;
        if !success {
            assert!(images.is_empty(), "Rejected proposal leaked preview images");
        }
        let last_rejected_attempt =
            if matches!(self.case.as_str(), "submission-limit" | "exhaustion-bypass") {
                925
            } else {
                922
            };
        if success {
            self.preview_sets += usize::from(!images.is_empty());
            let value: Value = serde_json::from_str(feedback)?;
            assert_eq!(value["validated"], true);
            assert_eq!(value["submission"], attempt - 919);
            assert_eq!(value["remainingSubmissions"], 925 - attempt);
            assert_eq!(value["remainingPreviewSets"], 3 - self.preview_sets);
            assert_eq!(
                images.is_empty(),
                self.proposal.as_ref().unwrap()["regions"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            self.accepted = true;
            if (self.case == "retry-schema-previews" && attempt < 925)
                || (self.case == "preview-limit" && attempt < 922)
            {
                let mut proposal = self.proposal.as_ref().unwrap().clone();
                proposal["regions"][0]["translation"][0]["bold"] = json!(true);
                proposal["regions"][0]["cleanupRects"][0]["width"] =
                    json!(if self.preview_sets == 1 { 0.3 } else { 0.4 });
                submit(output, thread, &proposal, attempt - 919)?;
                self.proposal = Some(proposal);
                return Ok(());
            }
            if self.case == "preview-limit" && attempt == 922 {
                let mut proposal = self.proposal.as_ref().unwrap().clone();
                proposal["regions"][0]["translation"][0]["bold"] = json!(false);
                proposal["regions"][0]["cleanupRects"][0]["width"] = json!(0.5);
                return submit(output, thread, &proposal, 3);
            }
            if self.case == "cancel-after-preview" {
                return Ok(());
            }
            if self.case == "repair-preview" && attempt == 920 {
                let correction = self.correction.take().unwrap();
                submit(output, thread, &correction, 1)?;
                self.proposal = Some(correction);
                return Ok(());
            }
            if self.case == "resubmit-accepted" && attempt == 920
                || self.case == "revise-invalid-latest" && attempt == 921
            {
                let mut proposal = self.proposal.as_ref().unwrap().clone();
                proposal["regions"] = json!([]);
                return submit(output, thread, &proposal, attempt - 919);
            }
            if matches!(
                self.case.as_str(),
                "revise-preview" | "revise-invalid-latest"
            ) && attempt == 920
            {
                let mut proposal = self.proposal.as_ref().unwrap().clone();
                proposal["regions"][0]["translation"][0]["bold"] = json!(true);
                proposal["regions"][0]["cleanupRects"][0]["width"] = json!(0.4);
                submit(output, thread, &proposal, 1)?;
                self.proposal = Some(proposal);
                return Ok(());
            }
            if self.case == "long-preview-budget" && attempt < 922 {
                return submit(
                    output,
                    thread,
                    self.proposal.as_ref().unwrap(),
                    attempt - 919,
                );
            }
        } else if self.case == "retry-schema-previews" {
            let expected = match attempt {
                920 => "Invalid translation proposal",
                921 => "cleanup rectangle",
                922 => "Overflow",
                _ => return Err("Unexpected submission rejection".into()),
            };
            assert!(feedback.contains(expected), "{feedback}");
            let mut proposal = host::proposal("ru");
            if attempt == 920 {
                proposal["regions"][0]["cleanupRects"][0]["x"] = json!(0.99);
            } else if attempt == 921 {
                let first = json!({"x":0.1,"y":0.1,"width":0.35,"height":0.35});
                proposal["regions"][0]["bounds"] = first.clone();
                proposal["regions"][0]["layout"] = first.clone();
                proposal["regions"][0]["cleanupRects"] = json!([first]);
                let mut sfx = proposal["regions"][0].clone();
                let second = json!({"x":0.55,"y":0.55,"width":0.35,"height":0.35});
                sfx["id"] = json!("unclear-sfx");
                sfx["sourceText"] = json!("ドン");
                sfx["bounds"] = second.clone();
                sfx["layout"] = second.clone();
                sfx["cleanupRects"] = json!([second]);
                proposal["regions"].as_array_mut().unwrap().push(sfx);
            }
            submit(output, thread, &proposal, attempt - 919)?;
            self.proposal = Some(proposal);
            return Ok(());
        } else if self.case == "preview-limit" {
            assert!(feedback.contains("native preview limit"), "{feedback}");
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
        } else if !self.accepted && attempt < last_rejected_attempt {
            return submit(
                output,
                thread,
                self.proposal.as_ref().unwrap(),
                attempt - 919,
            );
        } else if !self.accepted && attempt == 925 && self.case == "submission-limit" {
            return submit(output, thread, &host::proposal("ru"), 6);
        }
        let accepted = self.accepted || self.case == "exhaustion-bypass";
        let declined = matches!(
            self.case.as_str(),
            "final-rejected-after-acceptance" | "decline-long-reason"
        );
        host::finish(
            output,
            thread,
            json!({
                "accepted": accepted && !declined,
                "reason": if self.case == "decline-long-reason" {
                    concat!(
                        "Preview inspection revealed uncovered source strokes in the <b>engine sound</b> and large crash effect. ",
                        "No submissions remain to correct cleanup coverage. The dialogue can be translated, but placing the replacement ",
                        "sound effect would cover the car and nearby building details. I cannot approve this page while preserving its ",
                        "complete lettering and artwork. Original remains available; no cleanup image was generated. ",
                        "A thin pale halo would be acceptable; the uncovered source strokes remain readable."
                    ).into()
                } else if accepted && !declined {
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
