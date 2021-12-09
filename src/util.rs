/// Generate a name for a generated task.
///
/// # Arguments
///
/// * `parent_name` - Name of task parent task being generated.
/// * `task_index` - Index of sub-task being named.
/// * `total_tasks` - Total number of sub-tasks generated for this parent task.
/// * `variant` - Build Variant being generated.
pub fn name_generated_task(
    parent_name: &str,
    task_index: Option<u64>,
    total_tasks: Option<u64>,
    variant: Option<&str>,
) -> String {
    let suffix = if let Some(variant) = variant {
        format!("_{}", variant)
    } else {
        "".to_string()
    };

    if let Some(index) = task_index {
        let total_tasks = total_tasks.unwrap();
        let alignment = (total_tasks as f64).log10().ceil() as usize;
        format!(
            "{}_{:0fill$}{}",
            parent_name,
            index,
            suffix,
            fill = alignment
        )
    } else {
        format!("{}_misc{}", parent_name, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    #[rstest]
    #[case("task", Some(0), Some(10), None, "task_0")]
    #[case("task", Some(42), Some(1001), None, "task_0042")]
    #[case("task", None, Some(1001), None, "task_misc")]
    #[case("task", None, None, None, "task_misc")]
    #[case("task", Some(0), Some(10), Some("variant"), "task_0_variant")]
    #[case("task", Some(42), Some(1999), Some("variant"), "task_0042_variant")]
    #[case("task", None, None, Some("variant"), "task_misc_variant")]
    fn test_name_generated_task_should_not_include_suffix(
        #[case] name: &str,
        #[case] index: Option<u64>,
        #[case] total: Option<u64>,
        #[case] variant: Option<&str>,
        #[case] expected: &str,
    ) {
        let task_name = name_generated_task(name, index, total, variant);

        assert_eq!(task_name, expected);
    }
}
