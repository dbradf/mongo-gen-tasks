use cmd_lib::run_fun;

pub trait TestDiscovery {
    fn discover_tests(&self, suite: &str) -> Vec<String>;
}

pub struct ResmokeProxy {}

impl TestDiscovery for ResmokeProxy {
    fn discover_tests(&self, suite: &str) -> Vec<String> {
        let cmd_output = run_fun!(
            python buildscripts/resmoke.py discover --suite $suite
        )
        .unwrap();
        cmd_output.split("\n").map(|s| s.to_string()).collect()
    }
}
