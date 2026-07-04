    use super::*;

    #[test]
    fn provider_tool_descriptor_read_file_requires_path() {
        let descriptor = provider_tool_descriptor(ClientToolName::ReadTextFile);
        assert_eq!(descriptor["name"], "fs_read_text_file");
        assert_eq!(descriptor["parameters"]["required"], json!(["path"]));
        assert_eq!(descriptor["parameters"]["additionalProperties"], false);
        assert!(descriptor["parameters"]["properties"].get("path").is_some());
        assert!(descriptor["parameters"]["properties"].get("line").is_some());
        assert!(
            descriptor["parameters"]["properties"]
                .get("limit")
                .is_some()
        );
    }

    #[test]
    fn provider_tool_descriptor_find_paths_requires_glob() {
        let descriptor = provider_tool_descriptor(ClientToolName::FindPaths);
        assert_eq!(descriptor["name"], "fs_find_paths");
        assert_eq!(descriptor["parameters"]["required"], json!(["glob"]));
        assert_eq!(descriptor["parameters"]["additionalProperties"], false);
        assert!(descriptor["parameters"]["properties"].get("glob").is_some());
        assert!(descriptor["parameters"]["properties"].get("root").is_some());
        assert!(
            descriptor["parameters"]["properties"]
                .get("include_hidden")
                .is_some()
        );
    }

    #[test]
    fn provider_tool_descriptor_run_command_has_command_schema() {
        let descriptor = provider_tool_descriptor(ClientToolName::RunCommand);
        assert_eq!(descriptor["name"], "run_command");
        assert_eq!(descriptor["parameters"]["required"], json!(["command"]));
        assert_eq!(descriptor["parameters"]["additionalProperties"], false);
        assert!(
            descriptor["parameters"]["properties"]
                .get("command")
                .is_some()
        );
        assert!(descriptor["parameters"]["properties"].get("args").is_some());
        assert!(descriptor["parameters"]["properties"].get("cwd").is_some());
        assert!(descriptor["parameters"]["properties"]
            .get("bypass_tool_redirect")
            .is_some());
    }
