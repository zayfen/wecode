use wecode::CommandStep;

pub fn has_step(steps: &[CommandStep], expected: &CommandStep) -> bool {
    steps
        .iter()
        .any(|step| step.program == expected.program && step.args == expected.args)
}

pub fn has_openclaw_args(steps: &[CommandStep], expected_args: &[&str]) -> bool {
    steps.iter().any(|step| {
        step.program == "~/.wecode/openclaw-runtime/node_modules/.bin/openclaw"
            && step.args == expected_args
    })
}
