use crate::schema::models::VarCollection;
use crate::utils::ui::{info, print_header, success};
use anyhow::Result;

use super::shared::{write, Outcome, Target, WriteOptions};

/// Handle Blank mode: create minimal .env files
pub fn handle(target: &Target, opts: WriteOptions, _verbose: bool) -> Result<Outcome> {
    // A template already here and no stack named, under `--yes`: a fresh
    // clone. Keep it and create `.env` from it (DQ-31); it used to refuse.
    if opts.yes && !opts.force && target.template_path().exists() {
        return super::shared::set_up_from_template(target, opts);
    }

    if !opts.yes {
        print_header("Blank Template", Some("Creating minimal .env files"));
    }

    let example_content = "# Add your environment variables here\n# Format: KEY=value\n\n";
    let template_content = "# TODO: Replace with real values\n# Format: KEY=value\n\n";

    let name = target.template_name();
    write(
        target,
        &VarCollection::default(),
        example_content,
        template_content,
        opts,
        &|| {
            // Not "empty": the template carries two comment lines on purpose,
            // and a user who opens it after being told it is empty has reason
            // to distrust every other message (I-004).
            success(format!("Created {name} (no variables yet)"));
            info("Tip: Run 'evnx add' to add variables interactively");
        },
    )
}
