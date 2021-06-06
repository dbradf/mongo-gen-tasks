pub mod taskname;

pub struct SubSuite {
    pub index: usize,
    pub suite_name: String,
    pub test_list: Vec<String>,
}

pub struct GeneratedSuite {
    pub sub_suites: Vec<SubSuite>,
    pub build_variant: String,
    pub task_name: String,
    pub suite_name: String,
    pub filename: String,
    pub include_build_variant_in_name: bool,
}