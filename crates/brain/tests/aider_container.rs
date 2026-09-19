//! End-to-end contract test: the real `aider` (in a container) launched with
//! the exact env/args `brain bench` uses, talking to a mock OpenRouter on the
//! host. Asserts what reaches the endpoint: the Bearer key, the prefixed
//! model and the prompt. Needs Docker and the image from
//! `docker/aider-test.Dockerfile`:
//!
//!   docker build -f docker/aider-test.Dockerfile -t llm-brain-aider-test .
//!   cargo test -p brain --features docker-tests --test aider_container -- --ignored
#![cfg(feature = "docker-tests")]

use testcontainers::core::{ExecCommand, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const IMAGE_ENV: &str = "BRAIN_AIDER_IMAGE";

#[tokio::test]
#[ignore = "needs Docker and the llm-brain-aider-test image"]
async fn aider_in_container_hits_the_mock_endpoint_with_our_contract() {
    let image = std::env::var(IMAGE_ENV).unwrap_or_else(|_| "llm-brain-aider-test:latest".into());
    let (name, tag) = image.rsplit_once(':').unwrap_or((&image, "latest"));

    // Mock OpenRouter: one chat completion in aider's "whole" format is enough
    // for aider to finish cleanly; we only assert on the request.
    let server = MockServer::builder()
        .listener(std::net::TcpListener::bind("0.0.0.0:0").unwrap())
        .start()
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(bearer_token("sk-bench"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "gen-1", "object": "chat.completion", "created": 0, "model": "prism-ml/ternary-bonsai-2-27b",
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant",
                "content": "fixed.txt\n```\ndone\n```\n"}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        })))
        .expect(1..)
        .mount(&server)
        .await;
    let port = server.address().port();

    let container = GenericImage::new(name, tag)
        .with_wait_for(WaitFor::Nothing)
        .with_network("host")
        .start()
        .await
        .expect("start aider container (build docker/aider-test.Dockerfile first)");

    // a git repo inside the container, as `brain bench` would prepare a worktree
    let setup = ExecCommand::new([
        "sh",
        "-c",
        "git init -q /work/repo && cd /work/repo && echo bug > README.md && git add . && git commit -qm before",
    ]);
    container.exec(setup).await.expect("git setup");

    // Same env/args as bench::tool::invocation(Tool::Aider, …), pointed at the mock.
    let cmd = format!(
        "cd /work/repo && OPENAI_API_BASE=http://127.0.0.1:{port} OPENAI_API_KEY=sk-bench \
         aider --model openai/prism-ml/ternary-bonsai-2-27b --message 'create fixed.txt' \
         --yes-always --no-show-model-warnings --no-check-update --no-analytics --no-auto-commits --no-stream
         > /work/aider.log 2>&1; echo exit=$? >> /work/aider.log; tail -20 /work/aider.log"
    );
    let mut exec = container
        .exec(ExecCommand::new(["sh", "-c", &cmd]))
        .await
        .expect("run aider");
    let out = exec.stdout_to_vec().await.unwrap_or_default();
    let out = String::from_utf8_lossy(&out);
    eprintln!("aider output:\n{out}");

    let requests = server.received_requests().await.unwrap_or_default();
    let hit = requests
        .iter()
        .find(|r| r.url.path() == "/chat/completions")
        .expect("aider called the mock endpoint");
    let body: serde_json::Value = serde_json::from_slice(&hit.body).unwrap();
    assert_eq!(
        body["model"], "prism-ml/ternary-bonsai-2-27b",
        "model reaches OpenRouter without the openai/ prefix"
    );
    let text = body["messages"].to_string();
    assert!(
        text.contains("create fixed.txt"),
        "prompt forwarded: {text}"
    );
}
