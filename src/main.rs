use axum::{
    Router, extract::Json, http::StatusCode, response::Json as ResponseJson, routing::post,
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Debug, Deserialize, Serialize)]
struct BuildkiteWebhookEvent {
    event: String,
    build: Option<BuildkiteBuild>,
    job: Option<BuildkiteJob>,
    pipeline: Option<BuildkitePipeline>,
    agent: Option<BuildkiteAgent>,
    annotation: Option<BuildkiteAnnotation>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuildkiteBuild {
    id: Option<String>,
    number: Option<i32>,
    state: Option<String>,
    message: Option<String>,
    commit: Option<String>,
    branch: Option<String>,
    url: Option<String>,
    web_url: Option<String>,
    author: Option<BuildkiteAuthor>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuildkiteJob {
    id: Option<String>,
    name: Option<String>,
    command: Option<String>,
    state: Option<String>,
    exit_status: Option<i32>,
    web_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuildkitePipeline {
    id: Option<String>,
    name: Option<String>,
    slug: Option<String>,
    url: Option<String>,
    web_url: Option<String>,
    repository: Option<String>,
    provider: Option<BuildkiteProvider>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuildkiteAuthor {
    name: Option<String>,
    email: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuildkiteAgent {
    id: Option<String>,
    name: Option<String>,
    hostname: Option<String>,
    version: Option<String>,
    connection_state: Option<String>,
    ip_address: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuildkiteAnnotation {
    id: Option<String>,
    body: Option<String>,
    style: Option<String>,
    context: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuildkiteProvider {
    id: Option<String>,
    settings: Option<BuildkiteProviderSettings>,
    repository_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuildkiteProviderSettings {
    repository: Option<String>,
}

#[derive(Debug, Serialize)]
struct WebhookResponse {
    message: String,
}

#[derive(Clone)]
struct AppState {
    zulip_bot_email: String,
    zulip_bot_api_key: String,
    zulip_server_url: String,
    zulip_stream: String,
    client: reqwest::Client,
}

#[derive(Parser)]
#[command(name = "zulip-buildkite-bot")]
#[command(about = "A bot that forwards Buildkite events to Zulip")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the webhook server
    Server {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
        /// Zulip bot email
        #[arg(long, env = "ZULIP_BOT_EMAIL")]
        zulip_bot_email: String,
        /// Zulip bot API key
        #[arg(long, env = "ZULIP_BOT_API_KEY")]
        zulip_bot_api_key: String,
        /// Zulip server URL
        #[arg(long, env = "ZULIP_SERVER_URL")]
        zulip_server_url: String,
        /// Zulip stream/channel to post to
        #[arg(long, env = "ZULIP_STREAM")]
        zulip_stream: String,
    },
    /// Send test webhook events to a running server
    Test {
        /// Server URL to send test webhooks to
        #[arg(long, default_value = "http://localhost:3000")]
        server_url: String,
        /// Type of test event to send
        #[arg(long, default_value = "all")]
        event_type: String,
        /// Delay between events in seconds
        #[arg(long, default_value = "2")]
        delay: u64,
        /// Build number to use for test events
        #[arg(long, default_value = "123")]
        build_number: i32,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Server {
            port,
            zulip_bot_email,
            zulip_bot_api_key,
            zulip_server_url,
            zulip_stream,
        } => {
            tracing::info!("Starting webhook server on port {}", port);
            start_server(
                port,
                zulip_bot_email,
                zulip_bot_api_key,
                zulip_server_url,
                zulip_stream,
            )
            .await?;
        }
        Commands::Test {
            server_url,
            event_type,
            delay,
            build_number,
        } => {
            tracing::info!("Sending test webhook events to {}", server_url);
            run_tests(server_url, event_type, delay, build_number).await?;
        }
    }

    Ok(())
}

async fn start_server(
    port: u16,
    zulip_bot_email: String,
    zulip_bot_api_key: String,
    zulip_server_url: String,
    zulip_stream: String,
) -> anyhow::Result<()> {
    let state = AppState {
        zulip_bot_email,
        zulip_bot_api_key,
        zulip_server_url,
        zulip_stream,
        client: reqwest::Client::new(),
    };

    let app = Router::new()
        .route("/webhook", post(handle_webhook))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("Webhook server listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_webhook(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(payload): Json<BuildkiteWebhookEvent>,
) -> Result<ResponseJson<WebhookResponse>, StatusCode> {
    tracing::info!("Received webhook: {:?}", payload);

    let message_content = format_buildkite_message(&payload);
    
    // Don't send empty messages (filtered events)
    if message_content.trim().is_empty() {
        tracing::info!("Skipping filtered event: {}", payload.event);
        return Ok(ResponseJson(WebhookResponse {
            message: "Filtered".to_string(),
        }));
    }
    
    let topic = format_buildkite_topic(&payload);

    // Determine the target stream based on pipeline name
    let target_stream = determine_target_stream(&payload, &state.zulip_stream);

    match send_zulip_message(&state, &target_stream, &topic, &message_content).await {
        Ok(_) => {
            tracing::info!(
                "Successfully sent message to Zulip stream: {}",
                target_stream
            );
            Ok(ResponseJson(WebhookResponse {
                message: "OK".to_string(),
            }))
        }
        Err(e) => {
            tracing::error!("Failed to send message to Zulip: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn get_github_repo_url(pipeline: &BuildkitePipeline) -> Option<String> {
    // First try provider.repository_url (if it exists)
    if let Some(provider) = &pipeline.provider {
        if let Some(repo_url) = &provider.repository_url {
            return Some(repo_url.clone());
        }
        
        // Then try provider.settings.repository and convert to GitHub URL
        if let Some(settings) = &provider.settings {
            if let Some(repository) = &settings.repository {
                return Some(format!("https://github.com/{}", repository));
            }
        }
    }
    
    // Finally try pipeline.repository and convert it to GitHub URL
    if let Some(repository) = &pipeline.repository {
        // Handle git@github.com:owner/repo.git format
        if repository.starts_with("git@github.com:") {
            let repo_part = repository.strip_prefix("git@github.com:")
                .and_then(|s| s.strip_suffix(".git"))
                .unwrap_or(repository);
            return Some(format!("https://github.com/{}", repo_part));
        }
        // Handle https://github.com/owner/repo.git format
        if repository.starts_with("https://github.com/") {
            let repo_url = repository.strip_suffix(".git").unwrap_or(repository);
            return Some(repo_url.to_string());
        }
    }
    
    None
}

fn get_job_display_name(job: &BuildkiteJob) -> String {
    if let Some(name) = &job.name {
        if !name.trim().is_empty() {
            return name.clone();
        }
    }
    
    // Fallback to first line of command if name is not available
    if let Some(command) = &job.command {
        let first_line = command.lines().next().unwrap_or("");
        if !first_line.trim().is_empty() {
            // Truncate long commands to keep messages concise
            if first_line.len() > 40 {
                return format!("{}...", &first_line[..37]);
            }
            return first_line.to_string();
        }
    }
    
    "unnamed job".to_string()
}

fn determine_target_stream(event: &BuildkiteWebhookEvent, default_stream: &str) -> String {
    if let Some(ref pipeline) = event.pipeline {
        if let Some(ref name) = pipeline.name {
            let name_lower = name.to_lowercase();

            if name_lower.starts_with("lang-") || name_lower.starts_with("keyboard-") {
                let parts: Vec<&str> = name_lower.split('-').collect();
                if parts.len() >= 2 {
                    // Take the second part (after lang- or keyboard-)
                    return parts[1].to_string();
                }
            }
        }
    }

    // Default to the configured stream
    default_stream.to_string()
}

fn format_buildkite_message(event: &BuildkiteWebhookEvent) -> String {
    match event.event.as_str() {
        "build.started" => {
            if let Some(ref build) = event.build {
                let base_message = format!(
                    "🔄 Build [#{}]({}) started",
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#")
                );

                if let Some(ref message) = build.message {
                    if !message.trim().is_empty() {
                        // Add GitHub commit link if we have a commit SHA and repository URL
                        let commit_link = if let (Some(commit), Some(pipeline)) =
                            (&build.commit, &event.pipeline)
                        {
                            if let Some(repo_url) = get_github_repo_url(pipeline) {
                                let short_sha = commit.chars().take(7).collect::<String>();
                                format!(" ([{}]({}/commit/{}))", short_sha, repo_url, commit)
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };

                        format!("{}\n> {}{}", base_message, message, commit_link)
                    } else {
                        base_message
                    }
                } else {
                    base_message
                }
            } else {
                "🔄 Build started".to_string()
            }
        }
        "build.scheduled" => {
            if let Some(ref build) = event.build {
                let base_message = format!(
                    "📅 Build [#{}]({}) scheduled",
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#")
                );

                if let Some(ref message) = build.message {
                    if !message.trim().is_empty() {
                        // Add GitHub commit link if we have a commit SHA and repository URL
                        let commit_link = if let (Some(commit), Some(pipeline)) =
                            (&build.commit, &event.pipeline)
                        {
                            if let Some(repo_url) = get_github_repo_url(pipeline) {
                                let short_sha = commit.chars().take(7).collect::<String>();
                                format!(" ([{}]({}/commit/{}))", short_sha, repo_url, commit)
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };

                        format!("{}\n> {}{}", base_message, message, commit_link)
                    } else {
                        base_message
                    }
                } else {
                    base_message
                }
            } else {
                "📅 Build scheduled".to_string()
            }
        }
        "build.running" => {
            if let Some(ref build) = event.build {
                format!(
                    "🏃 Build [#{}]({}) running",
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#")
                )
            } else {
                "🏃 Build running".to_string()
            }
        }
        "build.blocked" => {
            if let Some(ref build) = event.build {
                format!(
                    "🚫 Build [#{}]({}) blocked",
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#")
                )
            } else {
                "🚫 Build blocked".to_string()
            }
        }
        "build.unblocked" => {
            if let Some(ref build) = event.build {
                format!(
                    "🟢 Build [#{}]({}) unblocked",
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#")
                )
            } else {
                "🟢 Build unblocked".to_string()
            }
        }
        "build.canceled" => {
            if let Some(ref build) = event.build {
                format!(
                    "⏹️ Build [#{}]({}) canceled",
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#")
                )
            } else {
                "⏹️ Build canceled".to_string()
            }
        }
        "build.created" => {
            if let Some(ref build) = event.build {
                let base_message = format!(
                    "🆕 Build [#{}]({}) created",
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#")
                );

                if let Some(ref message) = build.message {
                    if !message.trim().is_empty() {
                        // Add GitHub commit link if we have a commit SHA and repository URL
                        let commit_link = if let (Some(commit), Some(pipeline)) =
                            (&build.commit, &event.pipeline)
                        {
                            if let Some(repo_url) = get_github_repo_url(pipeline) {
                                let short_sha = commit.chars().take(7).collect::<String>();
                                format!(" ([{}]({}/commit/{}))", short_sha, repo_url, commit)
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };

                        format!("{}\n> {}{}", base_message, message, commit_link)
                    } else {
                        base_message
                    }
                } else {
                    base_message
                }
            } else {
                "🆕 Build created".to_string()
            }
        }
        "build.rebuilt" => {
            if let Some(ref build) = event.build {
                format!(
                    "🔁 Build [#{}]({}) rebuilt",
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#")
                )
            } else {
                "🔁 Build rebuilt".to_string()
            }
        }
        "build.finished" | "build.passed" | "build.failed" => {
            if let Some(ref build) = event.build {
                let (emoji, status_text) = match build.state.as_deref() {
                    Some("passed") => ("✅", "passed"),
                    Some("failed") => ("❌", "failed"),
                    Some("canceled") => ("⏹️", "canceled"),
                    _ => ("❓", "finished"),
                };

                format!(
                    "{} Build [#{}]({}) {}",
                    emoji,
                    build.number.unwrap_or(0),
                    build.web_url.as_deref().unwrap_or("#"),
                    status_text
                )
            } else {
                "✅ Build finished".to_string()
            }
        }
        "job.finished" => {
            if let Some(ref job) = event.job {
                let (emoji, status_text) = match job.exit_status {
                    Some(0) => return String::new(), // Don't forward successful jobs
                    Some(_) => ("❌", "failed"),
                    None => ("❓", "finished"),
                };

                format!(
                    "{} Job ['{}']({}) {}",
                    emoji,
                    get_job_display_name(job),
                    job.web_url.as_deref().unwrap_or("#"),
                    status_text
                )
            } else {
                String::new() // Don't forward if no job data
            }
        }
        "job.started" | "job.scheduled" | "job.canceled" | "job.retried" | "job.timed_out" | "job.assigned" => {
            // Don't forward any other job events
            String::new()
        }
        "agent.connected" => {
            if let Some(ref agent) = event.agent {
                format!(
                    "🟢 Agent '{}' connected ({})",
                    agent.name.as_deref().unwrap_or("unknown"),
                    agent.hostname.as_deref().unwrap_or("unknown host")
                )
            } else {
                "🟢 Agent connected".to_string()
            }
        }
        "agent.disconnected" => {
            if let Some(ref agent) = event.agent {
                format!(
                    "🔴 Agent '{}' disconnected ({})",
                    agent.name.as_deref().unwrap_or("unknown"),
                    agent.hostname.as_deref().unwrap_or("unknown host")
                )
            } else {
                "🔴 Agent disconnected".to_string()
            }
        }
        "annotation.created" => {
            if let Some(ref annotation) = event.annotation {
                let style_emoji = match annotation.style.as_deref() {
                    Some("success") => "✅",
                    Some("warning") => "⚠️",
                    Some("error") => "❌",
                    Some("info") => "ℹ️",
                    _ => "📝",
                };
                format!(
                    "{} Annotation created: {}",
                    style_emoji,
                    annotation.context.as_deref().unwrap_or("annotation")
                )
            } else {
                "📝 Annotation created".to_string()
            }
        }
        "annotation.updated" => {
            if let Some(ref annotation) = event.annotation {
                let style_emoji = match annotation.style.as_deref() {
                    Some("success") => "✅",
                    Some("warning") => "⚠️",
                    Some("error") => "❌",
                    Some("info") => "ℹ️",
                    _ => "📝",
                };
                format!(
                    "{} Annotation updated: {}",
                    style_emoji,
                    annotation.context.as_deref().unwrap_or("annotation")
                )
            } else {
                "📝 Annotation updated".to_string()
            }
        }
        "annotation.deleted" => {
            if let Some(ref annotation) = event.annotation {
                format!(
                    "🗑️ Annotation deleted: {}",
                    annotation.context.as_deref().unwrap_or("annotation")
                )
            } else {
                "🗑️ Annotation deleted".to_string()
            }
        }
        "pipeline.created" => {
            if let Some(ref pipeline) = event.pipeline {
                format!(
                    "🆕 Pipeline '{}' created",
                    pipeline.name.as_deref().unwrap_or("unknown")
                )
            } else {
                "🆕 Pipeline created".to_string()
            }
        }
        "pipeline.updated" => {
            if let Some(ref pipeline) = event.pipeline {
                format!(
                    "📝 Pipeline '{}' updated",
                    pipeline.name.as_deref().unwrap_or("unknown")
                )
            } else {
                "📝 Pipeline updated".to_string()
            }
        }
        "pipeline.deleted" => {
            if let Some(ref pipeline) = event.pipeline {
                format!(
                    "🗑️ Pipeline '{}' deleted",
                    pipeline.name.as_deref().unwrap_or("unknown")
                )
            } else {
                "🗑️ Pipeline deleted".to_string()
            }
        }
        _ => {
            format!("📢 Buildkite event: {}", event.event)
        }
    }
}

fn format_buildkite_topic(event: &BuildkiteWebhookEvent) -> String {
    if let Some(ref pipeline) = event.pipeline {
        format!("{} - Build", pipeline.name.as_deref().unwrap_or("Buildkite"))
    } else {
        "Build".to_string()
    }
}

async fn send_zulip_message(
    state: &AppState,
    target_stream: &str,
    topic: &str,
    content: &str,
) -> anyhow::Result<()> {
    let url = format!("{}/api/v1/messages", state.zulip_server_url);

    let form_data = [
        ("type", "stream"),
        ("to", target_stream),
        ("topic", topic),
        ("content", content),
    ];

    let response = state
        .client
        .post(&url)
        .basic_auth(&state.zulip_bot_email, Some(&state.zulip_bot_api_key))
        .form(&form_data)
        .send()
        .await?;

    if response.status().is_success() {
        tracing::debug!("Successfully sent message to Zulip");
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Failed to send message to Zulip: {} - {}", status, body);
    }
}

async fn run_tests(
    server_url: String,
    event_type: String,
    delay: u64,
    build_number: i32,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let webhook_url = format!("{}/webhook", server_url);

    let events = match event_type.as_str() {
        "build-started" => vec![create_mock_build_started(build_number)],
        "build-passed" => vec![create_mock_build_finished("passed", build_number)],
        "build-failed" => vec![create_mock_build_finished("failed", build_number)],
        "build-canceled" => vec![create_mock_build_finished("canceled", build_number)],
        "job-passed" => vec![create_mock_job_finished(0, build_number)],
        "job-failed" => vec![create_mock_job_finished(1, build_number)],
        "all" => vec![
            create_mock_build_started(build_number),
            create_mock_job_finished(0, build_number),
            create_mock_job_finished(1, build_number),
            create_mock_build_finished("passed", build_number),
        ],
        "scenario" => vec![
            create_mock_build_started(build_number),
            create_mock_job_finished(0, build_number),
            create_mock_job_finished(1, build_number),
            create_mock_build_finished("failed", build_number),
        ],
        "lang-routing" => vec![create_mock_lang_pipeline_event(build_number)],
        "keyboard-routing" => vec![create_mock_keyboard_pipeline_event(build_number)],
        _ => {
            anyhow::bail!(
                "Unknown event type: {}. Valid types: build-started, build-passed, build-failed, build-canceled, job-passed, job-failed, all, scenario, lang-routing, keyboard-routing",
                event_type
            );
        }
    };

    for (i, event) in events.iter().enumerate() {
        tracing::info!(
            "Sending test event {}/{}: {}",
            i + 1,
            events.len(),
            event.event
        );

        let response = client.post(&webhook_url).json(event).send().await?;

        if response.status().is_success() {
            tracing::info!("✅ Event sent successfully");
        } else {
            let status = response.status();
            let body = response.text().await?;
            tracing::error!("❌ Failed to send event: {} - {}", status, body);
        }

        if i < events.len() - 1 {
            tracing::info!("Waiting {} seconds before next event...", delay);
            tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
        }
    }

    tracing::info!("All test events sent!");
    Ok(())
}

fn create_mock_build_started(build_number: i32) -> BuildkiteWebhookEvent {
    BuildkiteWebhookEvent {
        event: "build.started".to_string(),
        build: Some(BuildkiteBuild {
            id: Some(format!("build-started-{}", build_number)),
            number: Some(build_number),
            state: Some("running".to_string()),
            message: Some("Add new feature for user authentication".to_string()),
            commit: Some("a1b2c3d4e5f6789012345678901234567890abcd".to_string()),
            branch: Some("feature/auth-improvements".to_string()),
            url: Some(format!(
                "https://api.buildkite.com/v2/organizations/my-org/pipelines/my-pipeline/builds/{}",
                build_number
            )),
            web_url: Some(format!(
                "https://buildkite.com/my-org/my-pipeline/builds/{}",
                build_number
            )),
            author: Some(BuildkiteAuthor {
                name: Some("Alice Developer".to_string()),
                email: Some("alice@example.com".to_string()),
            }),
        }),
        agent: None,
        annotation: None,
        job: None,
        pipeline: Some(BuildkitePipeline {
            id: Some("pipeline-123".to_string()),
            name: Some("My Awesome Pipeline".to_string()),
            slug: Some("my-awesome-pipeline".to_string()),
            url: Some(
                "https://api.buildkite.com/v2/organizations/my-org/pipelines/my-awesome-pipeline"
                    .to_string(),
            ),
            web_url: Some("https://buildkite.com/my-org/my-awesome-pipeline".to_string()),
            repository: Some("git@github.com:my-org/my-repo.git".to_string()),
            provider: Some(BuildkiteProvider {
                id: Some("github".to_string()),
                settings: Some(BuildkiteProviderSettings {
                    repository: Some("my-org/my-repo".to_string()),
                }),
                repository_url: Some("https://github.com/my-org/my-repo".to_string()),
            }),
        }),
    }
}

fn create_mock_build_finished(state: &str, build_number: i32) -> BuildkiteWebhookEvent {
    let commit_hash = match state {
        "passed" => "b2c3d4e5f6789012345678901234567890abcdef",
        "failed" => "c3d4e5f6789012345678901234567890abcdef12",
        "canceled" => "d4e5f6789012345678901234567890abcdef1234",
        _ => "unknown1234567890abcdef1234567890abcdef12",
    };

    let message = match state {
        "passed" => "Fix critical security vulnerability",
        "failed" => "Update dependencies to latest versions",
        "canceled" => "Refactor database connection handling",
        _ => "Unknown build message",
    };

    let author = match state {
        "passed" => "Bob Tester",
        "failed" => "Charlie Developer",
        "canceled" => "Dana Engineer",
        _ => "Unknown Author",
    };

    BuildkiteWebhookEvent {
        event: "build.finished".to_string(),
        build: Some(BuildkiteBuild {
            id: Some(format!("build-{}-{}", state, build_number)),
            number: Some(build_number),
            state: Some(state.to_string()),
            message: Some(message.to_string()),
            commit: Some(commit_hash.to_string()),
            branch: Some("main".to_string()),
            url: Some(format!(
                "https://api.buildkite.com/v2/organizations/my-org/pipelines/my-pipeline/builds/{}",
                build_number
            )),
            web_url: Some(format!(
                "https://buildkite.com/my-org/my-pipeline/builds/{}",
                build_number
            )),
            author: Some(BuildkiteAuthor {
                name: Some(author.to_string()),
                email: Some(format!(
                    "{}@example.com",
                    author.to_lowercase().replace(" ", ".")
                )),
            }),
        }),
        agent: None,
        annotation: None,
        job: None,
        pipeline: Some(BuildkitePipeline {
            id: Some("pipeline-123".to_string()),
            name: Some("My Awesome Pipeline".to_string()),
            slug: Some("my-awesome-pipeline".to_string()),
            url: Some(
                "https://api.buildkite.com/v2/organizations/my-org/pipelines/my-awesome-pipeline"
                    .to_string(),
            ),
            web_url: Some("https://buildkite.com/my-org/my-awesome-pipeline".to_string()),
            repository: Some("git@github.com:my-org/my-repo.git".to_string()),
            provider: Some(BuildkiteProvider {
                id: Some("github".to_string()),
                settings: Some(BuildkiteProviderSettings {
                    repository: Some("my-org/my-repo".to_string()),
                }),
                repository_url: Some("https://github.com/my-org/my-repo".to_string()),
            }),
        }),
    }
}

fn create_mock_job_finished(exit_status: i32, build_number: i32) -> BuildkiteWebhookEvent {
    let job_name = if exit_status == 0 {
        "Unit Tests"
    } else {
        "Linting"
    };
    let job_id = if exit_status == 0 {
        "job-tests-123"
    } else {
        "job-lint-456"
    };

    BuildkiteWebhookEvent {
        event: "job.finished".to_string(),
        build: None,
        job: Some(BuildkiteJob {
            id: Some(job_id.to_string()),
            name: Some(job_name.to_string()),
            command: Some("npm test".to_string()),
            state: Some(if exit_status == 0 { "passed" } else { "failed" }.to_string()),
            exit_status: Some(exit_status),
            web_url: Some(format!(
                "https://buildkite.com/my-org/my-pipeline/builds/{}#{}",
                build_number, job_id
            )),
        }),
        pipeline: Some(BuildkitePipeline {
            id: Some("pipeline-123".to_string()),
            name: Some("My Awesome Pipeline".to_string()),
            slug: Some("my-awesome-pipeline".to_string()),
            url: Some(
                "https://api.buildkite.com/v2/organizations/my-org/pipelines/my-awesome-pipeline"
                    .to_string(),
            ),
            web_url: Some("https://buildkite.com/my-org/my-awesome-pipeline".to_string()),
            repository: Some("git@github.com:my-org/my-repo.git".to_string()),
            provider: Some(BuildkiteProvider {
                id: Some("github".to_string()),
                settings: Some(BuildkiteProviderSettings {
                    repository: Some("my-org/my-repo".to_string()),
                }),
                repository_url: Some("https://github.com/my-org/my-repo".to_string()),
            }),
        }),
        agent: None,
        annotation: None,
    }
}

fn create_mock_lang_pipeline_event(build_number: i32) -> BuildkiteWebhookEvent {
    BuildkiteWebhookEvent {
        event: "build.started".to_string(),
        build: Some(BuildkiteBuild {
            id: Some(format!("lang-build-{}", build_number)),
            number: Some(build_number),
            state: Some("running".to_string()),
            message: Some("Update language pack translations".to_string()),
            commit: Some("lang123456789012345678901234567890abcd".to_string()),
            branch: Some("main".to_string()),
            url: Some(format!(
                "https://api.buildkite.com/v2/organizations/my-org/pipelines/lang-sami-x-private/builds/{}",
                build_number
            )),
            web_url: Some(format!(
                "https://buildkite.com/my-org/lang-sami-x-private/builds/{}",
                build_number
            )),
            author: Some(BuildkiteAuthor {
                name: Some("Language Team".to_string()),
                email: Some("lang@example.com".to_string()),
            }),
        }),
        job: None,
        pipeline: Some(BuildkitePipeline {
            id: Some("lang-pipeline-123".to_string()),
            name: Some("lang-sami-x-private".to_string()),
            slug: Some("lang-sami-x-private".to_string()),
            url: Some(
                "https://api.buildkite.com/v2/organizations/my-org/pipelines/lang-sami-x-private"
                    .to_string(),
            ),
            web_url: Some("https://buildkite.com/my-org/lang-sami-x-private".to_string()),
            repository: Some("git@github.com:my-org/my-repo.git".to_string()),
            provider: Some(BuildkiteProvider {
                id: Some("github".to_string()),
                settings: Some(BuildkiteProviderSettings {
                    repository: Some("my-org/my-repo".to_string()),
                }),
                repository_url: Some("https://github.com/my-org/my-repo".to_string()),
            }),
        }),
        agent: None,
        annotation: None,
    }
}

fn create_mock_keyboard_pipeline_event(build_number: i32) -> BuildkiteWebhookEvent {
    BuildkiteWebhookEvent {
        event: "build.started".to_string(),
        build: Some(BuildkiteBuild {
            id: Some(format!("keyboard-build-{}", build_number)),
            number: Some(build_number),
            state: Some("running".to_string()),
            message: Some("Update keyboard layout definitions".to_string()),
            commit: Some("kbd123456789012345678901234567890abcd".to_string()),
            branch: Some("main".to_string()),
            url: Some(format!("https://api.buildkite.com/v2/organizations/my-org/pipelines/keyboard-finnish-public/builds/{}", build_number)),
            web_url: Some(format!("https://buildkite.com/my-org/keyboard-finnish-public/builds/{}", build_number)),
            author: Some(BuildkiteAuthor {
                name: Some("Keyboard Team".to_string()),
                email: Some("keyboard@example.com".to_string()),
            }),
        }),
        agent: None,
        annotation: None,
        job: None,
        pipeline: Some(BuildkitePipeline {
            id: Some("keyboard-pipeline-123".to_string()),
            name: Some("keyboard-finnish-public".to_string()),
            slug: Some("keyboard-finnish-public".to_string()),
            url: Some("https://api.buildkite.com/v2/organizations/my-org/pipelines/keyboard-finnish-public".to_string()),
            web_url: Some("https://buildkite.com/my-org/keyboard-finnish-public".to_string()),
            repository: Some("git@github.com:my-org/my-repo.git".to_string()),
            provider: Some(BuildkiteProvider {
                id: Some("github".to_string()),
                settings: Some(BuildkiteProviderSettings {
                    repository: Some("my-org/my-repo".to_string()),
                }),
                repository_url: Some("https://github.com/my-org/my-repo".to_string()),
            }),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_build_started_message() {
        let event = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: Some(BuildkiteBuild {
                id: Some("abc123".to_string()),
                number: Some(42),
                state: None,
                message: Some("Add new feature for user authentication".to_string()),
                commit: Some("abcdef1234567890".to_string()),
                branch: Some("feature/auth-improvements".to_string()),
                url: Some("https://api.buildkite.com/v2/builds/123".to_string()),
                web_url: Some("https://buildkite.com/org/pipeline/builds/42".to_string()),
                author: Some(BuildkiteAuthor {
                    name: Some("Alice Developer".to_string()),
                    email: Some("alice@example.com".to_string()),
                }),
            }),
            agent: None,
            annotation: None,
            job: None,
            pipeline: Some(BuildkitePipeline {
                id: Some("pipeline123".to_string()),
                name: Some("My Pipeline".to_string()),
                slug: Some("my-pipeline".to_string()),
                url: Some("https://api.buildkite.com/v2/pipelines/123".to_string()),
                web_url: Some("https://buildkite.com/org/my-pipeline".to_string()),
                repository: None,
                provider: Some(BuildkiteProvider {
                    id: Some("github".to_string()),
                    settings: Some(BuildkiteProviderSettings {
                        repository: Some("my-org/my-repo".to_string()),
                    }),
                    repository_url: Some("https://github.com/my-org/my-repo".to_string()),
                }),
            }),
        };

        let message = format_buildkite_message(&event);
        assert!(
            message
                .contains("🔄 Build [#42](https://buildkite.com/org/pipeline/builds/42) started")
        );
        assert!(message.contains("> Add new feature for user authentication"));
        assert!(
            message
                .contains("([abcdef1](https://github.com/my-org/my-repo/commit/abcdef1234567890))")
        ); // GitHub commit link
        assert!(!message.contains("```spoiler")); // No spoiler details
        assert!(!message.contains("Details")); // No details section
    }

    #[test]
    fn test_format_build_started_no_message() {
        let event = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: Some(BuildkiteBuild {
                id: Some("abc123".to_string()),
                number: Some(42),
                state: None,
                message: None, // No commit message
                commit: Some("abcdef1234567890".to_string()),
                branch: Some("main".to_string()),
                url: Some("https://api.buildkite.com/v2/builds/123".to_string()),
                web_url: Some("https://buildkite.com/org/pipeline/builds/42".to_string()),
                author: Some(BuildkiteAuthor {
                    name: Some("Alice Developer".to_string()),
                    email: Some("alice@example.com".to_string()),
                }),
            }),
            agent: None,
            annotation: None,
            job: None,
            pipeline: None,
        };

        let message = format_buildkite_message(&event);
        assert_eq!(
            message,
            "🔄 Build [#42](https://buildkite.com/org/pipeline/builds/42) started"
        );
        assert!(!message.contains(">")); // No quote block when no message
    }

    #[test]
    fn test_format_build_finished_passed() {
        let event = BuildkiteWebhookEvent {
            event: "build.finished".to_string(),
            build: Some(BuildkiteBuild {
                id: Some("abc123".to_string()),
                number: Some(42),
                state: Some("passed".to_string()),
                message: Some("Fix the thing".to_string()),
                commit: Some("abcdef1234567890".to_string()),
                branch: Some("main".to_string()),
                url: Some("https://api.buildkite.com/v2/builds/123".to_string()),
                web_url: Some("https://buildkite.com/org/pipeline/builds/42".to_string()),
                author: Some(BuildkiteAuthor {
                    name: Some("John Doe".to_string()),
                    email: Some("john@example.com".to_string()),
                }),
            }),
            agent: None,
            annotation: None,
            job: None,
            pipeline: None,
        };

        let message = format_buildkite_message(&event);
        assert!(message.contains("✅ Build [#42]"));
        assert!(message.contains("passed"));
        assert!(!message.contains("```spoiler")); // No spoiler details
    }

    #[test]
    fn test_format_build_finished_failed() {
        let event = BuildkiteWebhookEvent {
            event: "build.finished".to_string(),
            build: Some(BuildkiteBuild {
                id: None,
                number: Some(42),
                state: Some("failed".to_string()),
                message: None,
                commit: None,
                branch: None,
                url: None,
                web_url: None,
                author: None,
            }),
            agent: None,
            annotation: None,
            job: None,
            pipeline: None,
        };

        let message = format_buildkite_message(&event);
        assert!(message.contains("❌ Build [#42]"));
        assert!(message.contains("failed"));
        assert!(!message.contains("```spoiler")); // No spoiler details
    }

    #[test]
    fn test_format_job_finished() {
        // Test failed job (should be forwarded)
        let failed_event = BuildkiteWebhookEvent {
            event: "job.finished".to_string(),
            build: None,
            job: Some(BuildkiteJob {
                id: Some("job123".to_string()),
                name: Some("Test Suite".to_string()),
                command: Some("npm test".to_string()),
                state: Some("failed".to_string()),
                exit_status: Some(1),
                web_url: Some("https://buildkite.com/org/pipeline/builds/42#job123".to_string()),
            }),
            agent: None,
            annotation: None,
            pipeline: None,
        };

        let message = format_buildkite_message(&failed_event);
        assert!(message.contains("❌ Job ['Test Suite']"));
        assert!(message.contains("failed"));
        assert!(message.contains("(https://buildkite.com/org/pipeline/builds/42#job123)"));

        // Test successful job (should be filtered out)
        let success_event = BuildkiteWebhookEvent {
            event: "job.finished".to_string(),
            build: None,
            job: Some(BuildkiteJob {
                id: Some("job123".to_string()),
                name: Some("Test Suite".to_string()),
                command: Some("npm test".to_string()),
                state: Some("passed".to_string()),
                exit_status: Some(0),
                web_url: Some("https://buildkite.com/org/pipeline/builds/42#job123".to_string()),
            }),
            agent: None,
            annotation: None,
            pipeline: None,
        };

        let success_message = format_buildkite_message(&success_event);
        assert!(success_message.is_empty()); // Should be empty (filtered)
    }

    #[test]
    fn test_format_buildkite_topic() {
        let event_with_pipeline_and_build = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: Some(BuildkiteBuild {
                id: None,
                number: Some(42),
                state: None,
                message: None,
                commit: None,
                branch: None,
                url: None,
                web_url: None,
                author: None,
            }),
            agent: None,
            annotation: None,
            job: None,
            pipeline: Some(BuildkitePipeline {
                id: None,
                name: Some("My Pipeline".to_string()),
                slug: None,
                url: None,
                web_url: None,
                repository: None,
                provider: None,
            }),
        };

        let topic = format_buildkite_topic(&event_with_pipeline_and_build);
        assert_eq!(topic, "My Pipeline - Build");

        let event_with_build_only = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: Some(BuildkiteBuild {
                id: None,
                number: Some(42),
                state: None,
                message: None,
                commit: None,
                branch: None,
                url: None,
                web_url: None,
                author: None,
            }),
            agent: None,
            annotation: None,
            job: None,
            pipeline: None,
        };

        let topic = format_buildkite_topic(&event_with_build_only);
        assert_eq!(topic, "Build");

        let event_with_pipeline_only = BuildkiteWebhookEvent {
            event: "job.finished".to_string(),
            build: None,
            job: None,
            pipeline: Some(BuildkitePipeline {
                id: None,
                name: Some("My Pipeline".to_string()),
                slug: None,
                url: None,
                web_url: None,
                repository: None,
                provider: None,
            }),
            agent: None,
            annotation: None,
        };

        let topic = format_buildkite_topic(&event_with_pipeline_only);
        assert_eq!(topic, "My Pipeline - Build");
    }

    #[test]
    fn test_determine_target_stream() {
        // Test lang- prefix
        let lang_event = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: None,
            job: None,
            pipeline: Some(BuildkitePipeline {
                id: None,
                name: Some("lang-foo-x-private".to_string()),
                slug: None,
                url: None,
                web_url: None,
                repository: None,
                provider: None,
            }),
            agent: None,
            annotation: None,
        };

        assert_eq!(determine_target_stream(&lang_event, "default"), "foo");

        // Test keyboard- prefix
        let keyboard_event = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: None,
            job: None,
            pipeline: Some(BuildkitePipeline {
                id: None,
                name: Some("keyboard-bar-public".to_string()),
                slug: None,
                url: None,
                web_url: None,
                repository: None,
                provider: None,
            }),
            agent: None,
            annotation: None,
        };

        assert_eq!(determine_target_stream(&keyboard_event, "default"), "bar");

        // Test case insensitive
        let mixed_case_event = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: None,
            job: None,
            pipeline: Some(BuildkitePipeline {
                id: None,
                name: Some("Lang-Baz-Something".to_string()),
                slug: None,
                url: None,
                web_url: None,
                repository: None,
                provider: None,
            }),
            agent: None,
            annotation: None,
        };

        assert_eq!(determine_target_stream(&mixed_case_event, "default"), "baz");

        // Test default fallback for other pipelines
        let other_event = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: None,
            job: None,
            pipeline: Some(BuildkitePipeline {
                id: None,
                name: Some("regular-pipeline".to_string()),
                slug: None,
                url: None,
                web_url: None,
                repository: None,
                provider: None,
            }),
            agent: None,
            annotation: None,
        };

        assert_eq!(
            determine_target_stream(&other_event, "buildkite"),
            "buildkite"
        );

        // Test no pipeline
        let no_pipeline_event = BuildkiteWebhookEvent {
            event: "build.started".to_string(),
            build: None,
            job: None,
            pipeline: None,
            agent: None,
            annotation: None,
        };

        assert_eq!(
            determine_target_stream(&no_pipeline_event, "buildkite"),
            "buildkite"
        );
    }

    #[test]
    fn test_format_job_scheduled_no_name() {
        // Note: job.scheduled events are now filtered out, so this should return empty
        let event = BuildkiteWebhookEvent {
            event: "job.scheduled".to_string(),
            build: None,
            job: Some(BuildkiteJob {
                id: Some("019884c5-7882-4b50-a31e-fdad05e19604".to_string()),
                name: None, // No name like in real Buildkite webhook
                command: Some("cargo build --bin box --release\nbuildkite-agent artifact upload target/release/box".to_string()),
                state: Some("scheduled".to_string()),
                exit_status: None,
                web_url: Some("https://buildkite.com/divvun/box/builds/3#019884c5-7882-4b50-a31e-fdad05e19604".to_string()),
            }),
            agent: None,
            annotation: None,
            pipeline: None,
        };

        let message = format_buildkite_message(&event);
        assert!(message.is_empty()); // Should be filtered out now
    }

    #[test]
    fn test_job_events_filtering() {
        // Test that non-failure job events are filtered out
        let scheduled_event = BuildkiteWebhookEvent {
            event: "job.scheduled".to_string(),
            build: None,
            job: Some(BuildkiteJob {
                id: Some("job123".to_string()),
                name: Some("Test Job".to_string()),
                command: Some("npm test".to_string()),
                state: Some("scheduled".to_string()),
                exit_status: None,
                web_url: Some("https://buildkite.com/org/pipeline/builds/42#job123".to_string()),
            }),
            agent: None,
            annotation: None,
            pipeline: None,
        };

        let message = format_buildkite_message(&scheduled_event);
        assert!(message.is_empty()); // Should be filtered out

        let started_event = BuildkiteWebhookEvent {
            event: "job.started".to_string(),
            build: None,
            job: Some(BuildkiteJob {
                id: Some("job123".to_string()),
                name: Some("Test Job".to_string()),
                command: Some("npm test".to_string()),
                state: Some("running".to_string()),
                exit_status: None,
                web_url: Some("https://buildkite.com/org/pipeline/builds/42#job123".to_string()),
            }),
            agent: None,
            annotation: None,
            pipeline: None,
        };

        let started_message = format_buildkite_message(&started_event);
        assert!(started_message.is_empty()); // Should be filtered out
    }

    #[test]
    fn test_format_build_scheduled_message() {
        let event = BuildkiteWebhookEvent {
            event: "build.scheduled".to_string(),
            build: Some(BuildkiteBuild {
                id: Some("abc123".to_string()),
                number: Some(42),
                state: Some("scheduled".to_string()),
                message: Some("Add new feature for user authentication".to_string()),
                commit: Some("abcdef1234567890".to_string()),
                branch: Some("feature/auth-improvements".to_string()),
                url: Some("https://api.buildkite.com/v2/builds/123".to_string()),
                web_url: Some("https://buildkite.com/org/pipeline/builds/42".to_string()),
                author: Some(BuildkiteAuthor {
                    name: Some("Alice Developer".to_string()),
                    email: Some("alice@example.com".to_string()),
                }),
            }),
            agent: None,
            annotation: None,
            job: None,
            pipeline: Some(BuildkitePipeline {
                id: Some("pipeline123".to_string()),
                name: Some("My Pipeline".to_string()),
                slug: Some("my-pipeline".to_string()),
                url: Some("https://api.buildkite.com/v2/pipelines/123".to_string()),
                web_url: Some("https://buildkite.com/org/my-pipeline".to_string()),
                repository: None,
                provider: Some(BuildkiteProvider {
                    id: Some("github".to_string()),
                    settings: Some(BuildkiteProviderSettings {
                        repository: Some("my-org/my-repo".to_string()),
                    }),
                    repository_url: Some("https://github.com/my-org/my-repo".to_string()),
                }),
            }),
        };

        let message = format_buildkite_message(&event);
        assert!(
            message
                .contains("📅 Build [#42](https://buildkite.com/org/pipeline/builds/42) scheduled")
        );
        assert!(message.contains("> Add new feature for user authentication"));
        assert!(
            message
                .contains("([abcdef1](https://github.com/my-org/my-repo/commit/abcdef1234567890))")
        ); // GitHub commit link
    }
}
