use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use noctrail_config::{AgentConfig, AgentProviderConfig, AgentProviderKind};
use serde_json::{Value as JsonValue, json};
use thiserror::Error;

const DEFAULT_LOCAL_ENDPOINT: &str = "http://127.0.0.1:11434/v1/responses";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAdapter {
    OpenAiCompatible(HttpProvider),
    Local(HttpProvider),
    Cli(CliProvider),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpProvider {
    label: &'static str,
    endpoint: String,
    model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliProvider {
    command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRequestPreview {
    pub kind: &'static str,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub command: Vec<String>,
    pub prompt_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderResponse {
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRisk {
    Low,
    Medium,
    High,
    Critical,
}

impl CommandRisk {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPermission {
    Review,
    StrongReview,
    Deny,
}

impl CommandPermission {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::StrongReview => "strong-review",
            Self::Deny => "deny",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "review" => Some(Self::Review),
            "strong-review" => Some(Self::StrongReview),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandProposal {
    pub command: String,
    pub reason: String,
    pub risk: CommandRisk,
    pub permission: CommandPermission,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchPreview {
    pub path: PathBuf,
    pub reason: String,
    pub diff: String,
}

#[derive(Debug, Error)]
pub enum CommandProposalError {
    #[error("command proposal payload was not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("command proposal payload must contain a non-empty proposals array")]
    MissingProposals,
    #[error("command proposal #{index} must contain a non-empty command")]
    MissingCommand { index: usize },
    #[error("command proposal #{index} must contain a non-empty reason")]
    MissingReason { index: usize },
    #[error("command proposal #{index} used an unknown risk level: {value}")]
    InvalidRisk { index: usize, value: String },
    #[error("command proposal #{index} used an unknown permission: {value}")]
    InvalidPermission { index: usize, value: String },
    #[error(
        "command proposal #{index} with {risk} risk must require strong-review or deny permission"
    )]
    StrongReviewRequired { index: usize, risk: &'static str },
}

#[derive(Debug, Error)]
pub enum PatchPreviewError {
    #[error("patch preview payload was not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("patch preview payload must contain a non-empty patches array")]
    MissingPatches,
    #[error("patch preview #{index} must contain a non-empty path")]
    MissingPath { index: usize },
    #[error("patch preview #{index} must contain a non-empty reason")]
    MissingReason { index: usize },
    #[error("patch preview #{index} must contain a unified diff")]
    MissingDiff { index: usize },
}

#[derive(Debug, Error)]
pub enum SuggestionError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Proposal(#[from] CommandProposalError),
    #[error(transparent)]
    Patch(#[from] PatchPreviewError),
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("agent provider is missing")]
    MissingProvider,
    #[error("invalid provider config: {0}")]
    InvalidConfig(String),
    #[error("http transport to {endpoint} failed: {message}")]
    HttpTransport { endpoint: String, message: String },
    #[error("http status {status} from {endpoint}: {body}")]
    HttpStatus {
        endpoint: String,
        status: u16,
        body: String,
    },
    #[error("provider response from {endpoint} was invalid JSON: {source}")]
    Json {
        endpoint: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("provider response from {endpoint} was missing output text")]
    MissingOutput { endpoint: String },
    #[error("failed to spawn CLI provider {program}: {source}")]
    CliSpawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write to CLI provider stdin for {program}: {source}")]
    CliStdin {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("CLI provider {program} exited with {code}: {stderr}")]
    CliExit {
        program: String,
        code: i32,
        stderr: String,
    },
    #[error("CLI provider output for {program} was not valid UTF-8: {source}")]
    CliUtf8 {
        program: String,
        #[source]
        source: std::string::FromUtf8Error,
    },
}

impl ProviderAdapter {
    pub fn from_agent_config(config: &AgentConfig) -> Result<Option<Self>, ProviderError> {
        if !config.enabled {
            return Ok(None);
        }

        let provider = config
            .provider
            .as_ref()
            .ok_or(ProviderError::MissingProvider)?;
        Ok(Some(Self::from_provider_config(provider)?))
    }

    pub fn from_provider_config(provider: &AgentProviderConfig) -> Result<Self, ProviderError> {
        match provider.kind {
            AgentProviderKind::OpenAiCompatible => Ok(Self::OpenAiCompatible(HttpProvider {
                label: "openai-compatible",
                endpoint: required_endpoint(provider, "openai-compatible")?,
                model: required_model(provider, "openai-compatible")?,
            })),
            AgentProviderKind::Local => Ok(Self::Local(HttpProvider {
                label: "local",
                endpoint: provider
                    .endpoint
                    .clone()
                    .unwrap_or_else(|| DEFAULT_LOCAL_ENDPOINT.to_string()),
                model: required_model(provider, "local")?,
            })),
            AgentProviderKind::Cli => Ok(Self::Cli(CliProvider {
                command: required_command(provider)?,
            })),
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::OpenAiCompatible(_) => "openai-compatible",
            Self::Local(_) => "local",
            Self::Cli(_) => "cli",
        }
    }

    pub fn request_preview(&self, prompt: &str) -> ProviderRequestPreview {
        match self {
            Self::OpenAiCompatible(provider) | Self::Local(provider) => ProviderRequestPreview {
                kind: provider.label,
                endpoint: Some(provider.endpoint.clone()),
                model: Some(provider.model.clone()),
                command: Vec::new(),
                prompt_chars: prompt.chars().count(),
            },
            Self::Cli(provider) => ProviderRequestPreview {
                kind: "cli",
                endpoint: None,
                model: None,
                command: provider.command.clone(),
                prompt_chars: prompt.chars().count(),
            },
        }
    }

    pub fn invoke(&self, prompt: &str) -> Result<ProviderResponse, ProviderError> {
        match self {
            Self::OpenAiCompatible(provider) | Self::Local(provider) => provider.invoke(prompt),
            Self::Cli(provider) => provider.invoke(prompt),
        }
    }

    pub fn propose_commands(&self, prompt: &str) -> Result<Vec<CommandProposal>, SuggestionError> {
        let response = self.invoke(prompt)?;
        CommandProposal::parse_many(&response.text).map_err(SuggestionError::from)
    }

    pub fn propose_patches(&self, prompt: &str) -> Result<Vec<PatchPreview>, SuggestionError> {
        let response = self.invoke(prompt)?;
        PatchPreview::parse_many(&response.text).map_err(SuggestionError::from)
    }
}

impl CommandProposal {
    pub fn parse_many(text: &str) -> Result<Vec<Self>, CommandProposalError> {
        let value = serde_json::from_str::<JsonValue>(text)
            .map_err(|source| CommandProposalError::Json { source })?;
        let raw_proposals = match &value {
            JsonValue::Array(proposals) => proposals,
            JsonValue::Object(object) => object
                .get("proposals")
                .and_then(JsonValue::as_array)
                .ok_or(CommandProposalError::MissingProposals)?,
            _ => return Err(CommandProposalError::MissingProposals),
        };
        if raw_proposals.is_empty() {
            return Err(CommandProposalError::MissingProposals);
        }

        raw_proposals
            .iter()
            .enumerate()
            .map(|(index, proposal)| Self::from_value(index, proposal))
            .collect()
    }

    fn from_value(index: usize, value: &JsonValue) -> Result<Self, CommandProposalError> {
        let Some(object) = value.as_object() else {
            return Err(CommandProposalError::MissingCommand { index });
        };
        let command = object
            .get("command")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(ToOwned::to_owned)
            .ok_or(CommandProposalError::MissingCommand { index })?;
        let reason = object
            .get("reason")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(ToOwned::to_owned)
            .ok_or(CommandProposalError::MissingReason { index })?;
        let risk_raw = object
            .get("risk")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| CommandProposalError::InvalidRisk {
                index,
                value: "<missing>".to_string(),
            })?;
        let risk =
            CommandRisk::parse(risk_raw).ok_or_else(|| CommandProposalError::InvalidRisk {
                index,
                value: risk_raw.to_string(),
            })?;
        let permission_raw = object
            .get("permission")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| CommandProposalError::InvalidPermission {
                index,
                value: "<missing>".to_string(),
            })?;
        let permission = CommandPermission::parse(permission_raw).ok_or_else(|| {
            CommandProposalError::InvalidPermission {
                index,
                value: permission_raw.to_string(),
            }
        })?;

        if matches!(risk, CommandRisk::High | CommandRisk::Critical)
            && !matches!(
                permission,
                CommandPermission::StrongReview | CommandPermission::Deny
            )
        {
            return Err(CommandProposalError::StrongReviewRequired {
                index,
                risk: risk.label(),
            });
        }

        Ok(Self {
            command,
            reason,
            risk,
            permission,
        })
    }
}

impl PatchPreview {
    pub fn parse_many(text: &str) -> Result<Vec<Self>, PatchPreviewError> {
        let value = serde_json::from_str::<JsonValue>(text)
            .map_err(|source| PatchPreviewError::Json { source })?;
        let raw_patches = match &value {
            JsonValue::Array(patches) => patches,
            JsonValue::Object(object) => object
                .get("patches")
                .and_then(JsonValue::as_array)
                .ok_or(PatchPreviewError::MissingPatches)?,
            _ => return Err(PatchPreviewError::MissingPatches),
        };
        if raw_patches.is_empty() {
            return Err(PatchPreviewError::MissingPatches);
        }

        raw_patches
            .iter()
            .enumerate()
            .map(|(index, patch)| Self::from_value(index, patch))
            .collect()
    }

    fn from_value(index: usize, value: &JsonValue) -> Result<Self, PatchPreviewError> {
        let Some(object) = value.as_object() else {
            return Err(PatchPreviewError::MissingPath { index });
        };
        let path = object
            .get("path")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .ok_or(PatchPreviewError::MissingPath { index })?;
        let reason = object
            .get("reason")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(ToOwned::to_owned)
            .ok_or(PatchPreviewError::MissingReason { index })?;
        let diff = object
            .get("diff")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|diff| !diff.is_empty() && looks_like_unified_diff(diff))
            .map(ToOwned::to_owned)
            .ok_or(PatchPreviewError::MissingDiff { index })?;

        Ok(Self { path, reason, diff })
    }
}

impl HttpProvider {
    fn invoke(&self, prompt: &str) -> Result<ProviderResponse, ProviderError> {
        let request = json!({
            "model": self.model,
            "input": prompt,
        });
        let response = match ureq::post(&self.endpoint)
            .set("Content-Type", "application/json")
            .send_json(request)
        {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                return Err(ProviderError::HttpStatus {
                    endpoint: self.endpoint.clone(),
                    status,
                    body,
                });
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(ProviderError::HttpTransport {
                    endpoint: self.endpoint.clone(),
                    message: error.to_string(),
                });
            }
        };

        let body = response
            .into_string()
            .map_err(|error| ProviderError::HttpTransport {
                endpoint: self.endpoint.clone(),
                message: error.to_string(),
            })?;
        let value =
            serde_json::from_str::<JsonValue>(&body).map_err(|source| ProviderError::Json {
                endpoint: self.endpoint.clone(),
                source,
            })?;
        let text = extract_output_text(&value).ok_or_else(|| ProviderError::MissingOutput {
            endpoint: self.endpoint.clone(),
        })?;
        Ok(ProviderResponse { text })
    }
}

impl CliProvider {
    fn invoke(&self, prompt: &str) -> Result<ProviderResponse, ProviderError> {
        let program = self.command.first().cloned().ok_or_else(|| {
            ProviderError::InvalidConfig("cli provider requires a command".to_string())
        })?;

        let mut child = Command::new(&program)
            .args(&self.command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| ProviderError::CliSpawn {
                program: program.clone(),
                source,
            })?;

        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(prompt.as_bytes())
                .map_err(|source| ProviderError::CliStdin {
                    program: program.clone(),
                    source,
                })?;
        }

        let output = child
            .wait_with_output()
            .map_err(|source| ProviderError::CliSpawn {
                program: program.clone(),
                source,
            })?;
        if !output.status.success() {
            return Err(ProviderError::CliExit {
                program,
                code: output.status.code().unwrap_or_default(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        let text = String::from_utf8(output.stdout)
            .map_err(|source| ProviderError::CliUtf8 { program, source })?;
        Ok(ProviderResponse { text })
    }
}

fn extract_output_text(value: &JsonValue) -> Option<String> {
    value
        .get("output_text")
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("choices")
                .and_then(JsonValue::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned)
        })
}

fn looks_like_unified_diff(diff: &str) -> bool {
    diff.starts_with("diff --git")
        || (diff.contains("\n--- ") && diff.contains("\n+++ "))
        || (diff.starts_with("--- ") && diff.contains("\n+++ "))
}

fn required_endpoint(
    provider: &AgentProviderConfig,
    label: &'static str,
) -> Result<String, ProviderError> {
    provider
        .endpoint
        .clone()
        .filter(|endpoint| !endpoint.trim().is_empty())
        .ok_or_else(|| {
            ProviderError::InvalidConfig(format!("{label} provider requires a non-empty endpoint"))
        })
}

fn required_model(
    provider: &AgentProviderConfig,
    label: &'static str,
) -> Result<String, ProviderError> {
    provider
        .model
        .clone()
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| {
            ProviderError::InvalidConfig(format!("{label} provider requires a non-empty model"))
        })
}

fn required_command(provider: &AgentProviderConfig) -> Result<Vec<String>, ProviderError> {
    if provider.command.is_empty() {
        return Err(ProviderError::InvalidConfig(
            "cli provider requires at least one command argument".to_string(),
        ));
    }
    Ok(provider.command.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn disabled_agent_returns_no_adapter() {
        let config = AgentConfig::default();

        assert_eq!(ProviderAdapter::from_agent_config(&config).unwrap(), None);
    }

    #[test]
    fn openai_provider_requires_endpoint_and_model() {
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::OpenAiCompatible,
            model: Some("gpt-5".to_string()),
            endpoint: None,
            command: Vec::new(),
        };

        let error = ProviderAdapter::from_provider_config(&provider).unwrap_err();
        assert!(error.to_string().contains("endpoint"));
    }

    #[test]
    fn local_provider_defaults_endpoint() {
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::Local,
            model: Some("llama".to_string()),
            endpoint: None,
            command: Vec::new(),
        };

        let adapter = ProviderAdapter::from_provider_config(&provider).unwrap();
        let preview = adapter.request_preview("prompt");
        assert_eq!(preview.kind, "local");
        assert_eq!(preview.endpoint.as_deref(), Some(DEFAULT_LOCAL_ENDPOINT));
        assert_eq!(preview.model.as_deref(), Some("llama"));
    }

    #[test]
    fn cli_provider_requires_command() {
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::Cli,
            model: None,
            endpoint: None,
            command: Vec::new(),
        };

        let error = ProviderAdapter::from_provider_config(&provider).unwrap_err();
        assert!(error.to_string().contains("command"));
    }

    #[test]
    fn http_provider_returns_output_text() {
        let (endpoint, handle) = spawn_fake_http_server(
            "HTTP/1.1 200 OK",
            json!({ "output_text": "provider ok" }).to_string(),
            Some("\"model\":\"gpt-5\""),
        );
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::OpenAiCompatible,
            model: Some("gpt-5".to_string()),
            endpoint: Some(endpoint),
            command: Vec::new(),
        };

        let adapter = ProviderAdapter::from_provider_config(&provider).unwrap();
        let response = adapter.invoke("hello provider").unwrap();
        handle.join().unwrap();
        assert_eq!(response.text, "provider ok");
    }

    #[test]
    fn http_provider_reports_status_errors() {
        let (endpoint, handle) = spawn_fake_http_server(
            "HTTP/1.1 500 Internal Server Error",
            "broken".to_string(),
            None,
        );
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::Local,
            model: Some("llama".to_string()),
            endpoint: Some(endpoint),
            command: Vec::new(),
        };

        let adapter = ProviderAdapter::from_provider_config(&provider).unwrap();
        let error = adapter.invoke("hello provider").unwrap_err();
        handle.join().unwrap();
        assert!(matches!(
            error,
            ProviderError::HttpStatus { status: 500, .. }
        ));
    }

    #[test]
    fn cli_provider_returns_stdout() {
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::Cli,
            model: None,
            endpoint: None,
            command: successful_cli_command(),
        };

        let adapter = ProviderAdapter::from_provider_config(&provider).unwrap();
        let response = adapter.invoke("hello provider").unwrap();
        assert!(response.text.contains("cli-provider-ok"));
    }

    #[test]
    fn cli_provider_nonzero_exit_is_reported() {
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::Cli,
            model: None,
            endpoint: None,
            command: failing_cli_command(),
        };

        let adapter = ProviderAdapter::from_provider_config(&provider).unwrap();
        let error = adapter.invoke("hello provider").unwrap_err();
        assert!(matches!(error, ProviderError::CliExit { .. }));
    }

    #[test]
    fn command_proposals_parse_reason_risk_and_permission() {
        let proposals = CommandProposal::parse_many(
            &json!({
                "proposals": [
                    {
                        "command": "git status",
                        "reason": "Inspect the repo before making changes.",
                        "risk": "low",
                        "permission": "review"
                    },
                    {
                        "command": "rm -rf build",
                        "reason": "Clean a broken build directory.",
                        "risk": "high",
                        "permission": "strong-review"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].risk, CommandRisk::Low);
        assert_eq!(proposals[0].permission, CommandPermission::Review);
        assert_eq!(proposals[1].risk, CommandRisk::High);
        assert_eq!(proposals[1].permission, CommandPermission::StrongReview);
    }

    #[test]
    fn high_risk_proposals_require_strong_review() {
        let error = CommandProposal::parse_many(
            &json!([
                {
                    "command": "sudo reboot",
                    "reason": "Restart the machine.",
                    "risk": "critical",
                    "permission": "review"
                }
            ])
            .to_string(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            CommandProposalError::StrongReviewRequired { .. }
        ));
    }

    #[test]
    fn propose_commands_runs_provider_and_parses_json_payload() {
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::Cli,
            model: None,
            endpoint: None,
            command: proposal_cli_command(),
        };

        let adapter = ProviderAdapter::from_provider_config(&provider).unwrap();
        let proposals = adapter.propose_commands("hello provider").unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].command, "git status");
        assert_eq!(proposals[0].permission, CommandPermission::Review);
    }

    #[test]
    fn patch_previews_parse_path_reason_and_diff() {
        let previews = PatchPreview::parse_many(
            &json!({
                "patches": [
                    {
                        "path": "src/lib.rs",
                        "reason": "Guard a missing check.",
                        "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n-foo\n+foo\n+bar\n"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].path, PathBuf::from("src/lib.rs"));
        assert!(previews[0].diff.contains("@@"));
    }

    #[test]
    fn patch_previews_require_unified_diff() {
        let error = PatchPreview::parse_many(
            &json!([
                {
                    "path": "src/lib.rs",
                    "reason": "Guard a missing check.",
                    "diff": "replace foo with bar"
                }
            ])
            .to_string(),
        )
        .unwrap_err();

        assert!(matches!(error, PatchPreviewError::MissingDiff { .. }));
    }

    #[test]
    fn propose_patches_runs_provider_and_parses_json_payload() {
        let provider = AgentProviderConfig {
            kind: AgentProviderKind::Cli,
            model: None,
            endpoint: None,
            command: patch_cli_command(),
        };

        let adapter = ProviderAdapter::from_provider_config(&provider).unwrap();
        let previews = adapter.propose_patches("hello provider").unwrap();
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].path, PathBuf::from("src/lib.rs"));
    }

    fn spawn_fake_http_server(
        status_line: &'static str,
        body: String,
        expected_fragment: Option<&'static str>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/responses", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            let mut expected_len = None;

            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);

                if expected_len.is_none()
                    && let Some(header_end) =
                        request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end]).to_string();
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            if name.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    expected_len = Some(header_end + 4 + content_length);
                }

                if expected_len.is_some_and(|expected_len| request.len() >= expected_len) {
                    break;
                }
            }

            let request = String::from_utf8_lossy(&request).to_string();
            if let Some(fragment) = expected_fragment {
                assert!(request.contains(fragment), "{request}");
            }
            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });
        (endpoint, handle)
    }

    fn successful_cli_command() -> Vec<String> {
        #[cfg(windows)]
        {
            vec![
                "cmd".to_string(),
                "/C".to_string(),
                "echo cli-provider-ok".to_string(),
            ]
        }

        #[cfg(not(windows))]
        {
            vec![
                "sh".to_string(),
                "-lc".to_string(),
                "printf cli-provider-ok".to_string(),
            ]
        }
    }

    fn failing_cli_command() -> Vec<String> {
        #[cfg(windows)]
        {
            vec![
                "cmd".to_string(),
                "/C".to_string(),
                "echo provider-fail 1>&2 & exit /b 17".to_string(),
            ]
        }

        #[cfg(not(windows))]
        {
            vec![
                "sh".to_string(),
                "-lc".to_string(),
                "printf provider-fail >&2; exit 17".to_string(),
            ]
        }
    }

    fn proposal_cli_command() -> Vec<String> {
        let payload = json!([
            {
                "command": "git status",
                "reason": "Inspect the repository state.",
                "risk": "low",
                "permission": "review"
            }
        ])
        .to_string();

        #[cfg(windows)]
        {
            vec![
                "cmd".to_string(),
                "/C".to_string(),
                format!("echo {payload}"),
            ]
        }

        #[cfg(not(windows))]
        {
            vec![
                "sh".to_string(),
                "-lc".to_string(),
                format!("printf '%s' '{payload}'"),
            ]
        }
    }

    fn patch_cli_command() -> Vec<String> {
        let payload = json!([
            {
                "path": "src/lib.rs",
                "reason": "Guard a missing check.",
                "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n-foo\n+foo\n+bar\n"
            }
        ])
        .to_string();

        #[cfg(windows)]
        {
            vec![
                "cmd".to_string(),
                "/C".to_string(),
                format!("echo {payload}"),
            ]
        }

        #[cfg(not(windows))]
        {
            vec![
                "sh".to_string(),
                "-lc".to_string(),
                format!("printf '%s' '{payload}'"),
            ]
        }
    }
}
