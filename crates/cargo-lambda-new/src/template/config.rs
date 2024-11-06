use cargo_lambda_interactive::{
    validator::{ErrorMessage, Validation},
    Confirm, CustomUserError, Text,
};
use indexmap::IndexMap;
use liquid::{model::Value, Object};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum PromptValue {
    Boolean(bool),
    String(String),
}

impl PromptValue {
    pub fn to_value(&self) -> Value {
        match self {
            PromptValue::Boolean(b) => Value::scalar(*b),
            PromptValue::String(s) => Value::scalar(s.clone()),
        }
    }
}

impl Default for PromptValue {
    fn default() -> Self {
        PromptValue::String(String::default())
    }
}

impl From<PromptValue> for Value {
    fn from(value: PromptValue) -> Self {
        value.to_value()
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RenderCondition {
    pub var: String,
    pub r#match: Option<PromptValue>,
    pub not_match: Option<PromptValue>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct TemplatePrompt {
    pub message: String,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub default: Option<PromptValue>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct TemplateConfig {
    #[serde(default)]
    pub disable_default_prompts: bool,
    #[serde(default)]
    pub prompts: IndexMap<String, TemplatePrompt>,
    #[serde(default)]
    pub render_files: Vec<PathBuf>,
    #[serde(default)]
    pub render_all_files: bool,
    #[serde(default)]
    pub ignore_files: Vec<PathBuf>,
    #[serde(default)]
    pub render_conditional_files: HashMap<String, RenderCondition>,
    #[serde(default)]
    pub ignore_conditional_files: HashMap<String, RenderCondition>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CargoLambdaConfig {
    pub template: TemplateConfig,
}

#[tracing::instrument(target = "cargo_lambda")]
pub(crate) fn parse_template_config<P: AsRef<Path> + Debug>(path: P) -> Result<TemplateConfig> {
    let config_path = path.as_ref().join("CargoLambda.toml");
    if !config_path.exists() {
        return Ok(TemplateConfig::default());
    }

    let contents = fs::read_to_string(config_path)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to read CargoLambda.toml at {:?}", path.as_ref()))?;

    let config: CargoLambdaConfig = toml::from_str(&contents)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to parse CargoLambda.toml at {:?}", path.as_ref()))?;

    Ok(config.template)
}

impl TemplateConfig {
    pub(crate) fn ask_template_options(&self, no_interactive: bool) -> Result<Object> {
        let mut variables = Object::new();
        for (name, prompt) in &self.prompts {
            let value = if no_interactive {
                prompt.default.clone().unwrap_or_default()
            } else {
                prompt.ask()?
            };
            variables.insert(name.into(), value.into());
        }
        Ok(variables)
    }
}

impl TemplatePrompt {
    pub(crate) fn ask(&self) -> Result<PromptValue> {
        match &self.default {
            Some(PromptValue::Boolean(b)) => {
                let value = Confirm::new(&self.message)
                    .with_default(*b)
                    .prompt()
                    .into_diagnostic()?;
                Ok(PromptValue::Boolean(value))
            }
            Some(PromptValue::String(s)) => {
                let value = self
                    .text_prompt()
                    .with_default(s)
                    .prompt()
                    .into_diagnostic()?;
                Ok(PromptValue::String(value))
            }
            None => {
                let value = self.text_prompt().prompt().into_diagnostic()?;
                Ok(PromptValue::String(value))
            }
        }
    }

    fn text_prompt(&self) -> Text {
        let mut prompt = Text::new(&self.message);

        if let Some(choices) = &self.choices {
            let choices_for_suggest = choices.clone();
            let choices_for_validator = choices.clone();

            let autocomplete = move |input: &str| suggest_choice(input, &choices_for_suggest);
            let validator = move |input: &str| validate_choice(input, &choices_for_validator);

            prompt = prompt.with_autocomplete(autocomplete);
            prompt = prompt.with_validator(validator);
        }

        prompt
    }
}

fn suggest_choice(input: &str, choices: &[String]) -> Result<Vec<String>, CustomUserError> {
    Ok(choices
        .iter()
        .filter_map(|s| {
            if s.starts_with(input) {
                Some(s.to_string())
            } else {
                None
            }
        })
        .collect())
}

fn validate_choice(input: &str, choices: &[String]) -> Result<Validation, CustomUserError> {
    if choices.contains(&input.to_string()) {
        Ok(Validation::Valid)
    } else {
        Ok(Validation::Invalid(ErrorMessage::Custom(format!(
            "invalid choice: {input}"
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_missing_template_config() {
        let config = parse_template_config("../../tests/templates/function-template").unwrap();
        assert_eq!(config.prompts.len(), 0);
    }

    #[test]
    fn test_parse_template_config_prompts() {
        let config = parse_template_config("../../tests/templates/config-template").unwrap();
        assert_eq!(config.disable_default_prompts, true);
        assert_eq!(config.prompts.len(), 9);

        assert_eq!(
            config.prompts["project_description"].message,
            "What is the description of your project?"
        );
        assert_eq!(
            config.prompts["project_description"].default,
            Some(PromptValue::String("My Lambda".to_string()))
        );
        assert_eq!(config.prompts["project_description"].choices, None);

        assert_eq!(
            config.prompts["enable_tracing"].message,
            "Would you like to enable tracing?"
        );
        assert_eq!(
            config.prompts["enable_tracing"].default,
            Some(PromptValue::Boolean(false))
        );
        assert_eq!(config.prompts["enable_tracing"].choices, None);

        assert_eq!(
            config.prompts["runtime"].message,
            "Which runtime would you like to use?"
        );
        assert_eq!(
            config.prompts["runtime"].default,
            Some(PromptValue::String("provided.al2023".to_string()))
        );
        assert_eq!(
            config.prompts["runtime"].choices,
            Some(vec![
                "provided.al2023".to_string(),
                "provided.al2".to_string()
            ])
        );
    }

    #[test]
    fn test_parse_template_config_render_files() {
        let config = parse_template_config("../../tests/templates/config-template").unwrap();
        assert_eq!(
            config.render_files,
            vec!["Cargo.toml", "README.md", "main.rs"]
                .iter()
                .map(|s| PathBuf::from(s))
                .collect::<Vec<PathBuf>>()
        );
        assert!(config.render_all_files);
    }

    #[test]
    fn test_parse_template_config_ignore_files() {
        let config = parse_template_config("../../tests/templates/config-template").unwrap();
        assert_eq!(
            config.ignore_files,
            vec!["README.md"]
                .iter()
                .map(|s| PathBuf::from(s))
                .collect::<Vec<PathBuf>>()
        );
    }

    #[test]
    fn test_validate_choice() {
        let choices = vec!["a".to_string(), "b".to_string()];
        assert_eq!(validate_choice("a", &choices).unwrap(), Validation::Valid);
        assert_eq!(validate_choice("b", &choices).unwrap(), Validation::Valid);
        assert_eq!(
            validate_choice("c", &choices).unwrap(),
            Validation::Invalid(ErrorMessage::Custom("invalid choice: c".to_string()))
        );
    }

    #[test]
    fn test_suggest_choice() {
        let choices = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            suggest_choice("a", &choices).unwrap(),
            vec!["a".to_string()]
        );
        assert_eq!(
            suggest_choice("b", &choices).unwrap(),
            vec!["b".to_string()]
        );
    }

    #[test]
    fn test_ask_template_options() {
        let config = parse_template_config("../../tests/templates/config-template").unwrap();
        let variables = config.ask_template_options(true).unwrap();
        assert_eq!(variables.len(), 9);

        assert_eq!(variables["project_description"], "My Lambda");
        assert_eq!(variables["enable_tracing"], false);
        assert_eq!(variables["runtime"], "provided.al2023");
        assert_eq!(variables["architecture"], "x86_64");
        assert_eq!(variables["memory"], "128");
        assert_eq!(variables["timeout"], "3");
        assert_eq!(variables["github_actions"], false);
        assert_eq!(variables["ci_provider"], ".github");
        assert_eq!(variables["license"], "Ignore license");
    }

    #[test]
    fn test_parse_template_config_render_conditions() {
        let config = parse_template_config("../../tests/templates/config-template").unwrap();
        assert_eq!(config.render_conditional_files.len(), 1);
        assert_eq!(
            config.render_conditional_files[".github"].var,
            "github_actions"
        );
        assert_eq!(
            config.render_conditional_files[".github"].r#match,
            Some(PromptValue::Boolean(true))
        );
    }

    #[test]
    fn test_parse_template_config_ignore_conditions() {
        let config = parse_template_config("../../tests/templates/config-template").unwrap();
        assert_eq!(config.ignore_conditional_files.len(), 2);
        assert_eq!(config.ignore_conditional_files["Apache.txt"].var, "license");
        assert_eq!(
            config.ignore_conditional_files["Apache.txt"].not_match,
            Some(PromptValue::String("APACHE".to_string()))
        );
        assert_eq!(config.ignore_conditional_files["MIT.txt"].var, "license");
        assert_eq!(
            config.ignore_conditional_files["MIT.txt"].not_match,
            Some(PromptValue::String("MIT".to_string()))
        );
    }
}
