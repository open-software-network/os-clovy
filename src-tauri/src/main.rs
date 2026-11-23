#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let arguments: Vec<_> = std::env::args_os().collect();
        if arguments
            .get(1)
            .is_some_and(|argument| argument == "--computer-use-release-self-test-host")
        {
            let Some(driver) = arguments.get(2) else {
                eprintln!("Computer use self-test host requires the bundled helper path.");
                std::process::exit(64);
            };
            let permission_prompt = arguments.get(3).and_then(|argument| {
                argument
                    .to_str()
                    .and_then(|value| value.strip_prefix("--permission-prompt="))
                    .map(str::to_string)
            });
            if arguments.len() > 4 || (arguments.len() == 4 && permission_prompt.is_none()) {
                eprintln!("Computer use self-test host received unexpected arguments.");
                std::process::exit(64);
            }
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Computer use self-test runtime should start");
            if let Err(error) =
                runtime.block_on(clovy_lib::computer_use::run_release_self_test_host(
                    std::path::PathBuf::from(driver),
                    permission_prompt,
                ))
            {
                eprintln!("Computer use self-test host stopped: {error}");
                std::process::exit(1);
            }
            return;
        }
    }
    clovy_lib::run();
}
