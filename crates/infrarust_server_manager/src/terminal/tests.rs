#[cfg(test)]
mod tests {
    use crate::execute_command;

    #[test]
    fn test_execute_command_echo() {
        #[cfg(target_os = "windows")]
        let command = "echo Hello, World!";
        #[cfg(target_os = "linux")]
        let command = "echo 'Hello, World!'";

        let result = execute_command(command);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("Hello, World!"));
    }

    #[test]
    fn test_execute_command_error() {
        let command = "thiscommandshouldfail12345";

        let result = execute_command(command);
        assert!(result.is_err());
    }
}
