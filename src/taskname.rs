const GEN_SUFFIX: &str = "_gen";

pub fn name_sub_suite(
    parent_name: &str,
    task_index: u64,
    total_tasks: u64,
    build_variant: Option<&str>,
) -> String {
    let suffix = if let Some(bv) = build_variant {
        format!("_{}", bv)
    } else {
        String::from("")
    };

    let width = (total_tasks as f64).log10().ceil() as usize;

    format!(
        "{}_{:0fill$}{}",
        parent_name,
        task_index,
        suffix,
        fill = width
    )
}

pub fn remove_gen_suffix(task_name: &str) -> String {
    if task_name.ends_with(GEN_SUFFIX) {
        let end = task_name.len() - GEN_SUFFIX.len();
        String::from(&task_name[..end])
    } else {
        String::from(task_name)
    }
}

pub fn remove_gen_suffix_ref(task_name: &str) -> &str {
    if task_name.ends_with(GEN_SUFFIX) {
        let end = task_name.len() - GEN_SUFFIX.len();
        &task_name[..end]
    } else {
        task_name
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_name_sub_suite() {
        assert_eq!(name_sub_suite("task", 0, 5, None), "task_0");
        assert_eq!(name_sub_suite("task", 3, 15, None), "task_03");
        assert_eq!(name_sub_suite("task", 42, 314, None), "task_042");
        assert_eq!(
            name_sub_suite("hello", 42, 314, Some("world")),
            "hello_042_world"
        );
    }

    #[test]
    fn test_remove_gen_suffix() {
        assert_eq!(remove_gen_suffix("task_name"), "task_name");
        assert_eq!(remove_gen_suffix("task_name_gen"), "task_name");
        assert_eq!(remove_gen_suffix("task_name_"), "task_name_");
    }
}
