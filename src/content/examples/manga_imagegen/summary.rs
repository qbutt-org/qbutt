//! Measured production summaries through owned no-inference process/SQLite paths.

use std::process::Command;

use serde_json::Value;

use super::{Result, SOURCE_TEXT};

pub(super) fn exercise() -> Result<()> {
    for (case, tools, submits, evaluated, previews, outcome, cached) in [
        ("invented-exhaustion", 0, 0, 0, 0, "declined", 0),
        ("final-rejected-after-acceptance", 2, 1, 1, 1, "declined", 0),
        ("decline-long-reason", 2, 1, 1, 1, "declined", 0),
        ("invalid-geometry", 4, 3, 3, 0, "declined", 0),
        ("submission-limit", 5, 4, 3, 0, "declined", 0),
        ("final-bypass", 1, 0, 0, 0, "missing_candidate", 0),
        ("exhaustion-bypass", 4, 3, 3, 0, "missing_candidate", 0),
        ("cancel-after-preview", 2, 1, 1, 1, "cancelled", 0),
        ("cancel-prepare", 2, 1, 1, 0, "cancelled", 0),
        ("retry-geometry", 3, 2, 2, 1, "accepted", 1),
        ("resubmit-accepted", 3, 2, 2, 1, "accepted", 1),
        ("revise-invalid-latest", 4, 3, 3, 2, "accepted", 1),
    ] {
        let mut command = Command::new(std::env::current_exe()?);
        command.args(["--summary-case", case]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let output = command.output()?;
        let stderr = String::from_utf8(output.stderr)?;
        let stdout = String::from_utf8(output.stdout)?;
        if !output.status.success() {
            return Err(format!("{case}: {stdout} {stderr}").into());
        }
        let summaries: Vec<Value> = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("MANGA_SUBMISSION_SUMMARY "))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        assert_eq!(summaries.len(), 1, "{case}: {stderr}");
        let summary = &summaries[0];
        assert_eq!(summary["toolCalls"], tools, "{case}");
        assert_eq!(summary["contextCalls"], tools - submits, "{case}");
        assert_eq!(summary["submitCalls"], submits, "{case}");
        assert_eq!(summary["evaluatedSubmissions"], evaluated, "{case}");
        assert_eq!(summary["returnedPreviewImages"], previews, "{case}");
        assert_eq!(summary["outcome"], outcome, "{case}");
        assert_eq!(summary["limitRejections"], submits - evaluated, "{case}");
        let decision = match outcome {
            "cancelled" => Value::Null,
            "declined" => Value::Bool(false),
            _ => Value::Bool(true),
        };
        assert_eq!(summary["modelDecision"], decision, "{case}");
        assert_eq!(
            summary["submissions"].as_array().unwrap().len(),
            evaluated as usize
        );
        assert_eq!(
            summary["returnedPreviewBytes"].as_u64().unwrap() > 0,
            previews > 0
        );
        if case == "invalid-geometry" {
            for rejected in summary["submissions"].as_array().unwrap() {
                assert_eq!(rejected["outcome"], "rejected");
                assert_eq!(rejected["reason"], "region geometry or style validation");
            }
        }
        let encoded = serde_json::to_string(summary)?;
        assert!(!encoded.contains(SOURCE_TEXT));
        assert!(!encoded.contains("レン"));
        assert!(!encoded.contains("invented"));
        assert!(!encoded.contains("The model declined"));
        assert!(
            !encoded.contains("sourceText")
                && !encoded.contains("translation")
                && !encoded.contains("terms")
        );
        let cache: Value = stdout
            .lines()
            .find_map(|line| line.strip_prefix("MANGA_SUMMARY_CACHE "))
            .map(serde_json::from_str)
            .transpose()?
            .ok_or("cache receipt missing")?;
        assert_eq!(cache["pngRows"], cached, "{case}");
        assert_eq!(cache["contextRows"], cached, "{case}");
        println!(
            "PASS manga_summary case={case} tools={tools} submits={submits} previews={previews} outcome={outcome} png_context_rows={cached}"
        );
    }
    Ok(())
}
