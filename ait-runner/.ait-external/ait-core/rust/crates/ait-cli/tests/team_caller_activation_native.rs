//! Native active-writer verification uses an isolated home and a local model fixture.
#![cfg(unix)]
use ait_cli::init_surface::{init_repo, InitRequest};
use ait_core::json_support::{json, JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

struct Host {
    child: Child,
    input: ChildStdin,
    output: Receiver<JsonValue>,
    next: u64,
}
impl Host {
    fn new(home: &Path, url: &str) -> Self {
        let mut child=Command::new("codex").env("CODEX_HOME",home)
            .args(["-c","model_provider=\"fixture\"","-c",&format!("model_providers.fixture={{name=\"Local fixture\",base_url=\"{url}\",wire_api=\"responses\",requires_openai_auth=false}}"),"app-server","--stdio"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let value = JsonCodec::parse_value_with_error_prefix(&line, "native RPC").unwrap();
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        let mut host = Self {
            child,
            input,
            output,
            next: 0,
        };
        let init=host.rpc("initialize",json!({"clientInfo":{"name":"ait_caller_fixture","version":"1"},"capabilities":{"experimentalApi":true,"explicitGatewayOauth":true}}));
        assert!(init.get("error").is_none(), "{init}");
        writeln!(host.input, "{}", json!({"method":"initialized"})).unwrap();
        host.input.flush().unwrap();
        host
    }
    fn rpc(&mut self, method: &str, params: JsonValue) -> JsonValue {
        self.next += 1;
        let id = self.next;
        writeln!(
            self.input,
            "{}",
            json!({"id":id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        loop {
            let reply = self
                .output
                .recv_timeout(Duration::from_secs(15))
                .expect("native RPC timeout");
            if reply["id"] == id {
                return reply;
            }
            assert!(
                !(reply.get("method").is_some() && reply.get("id").is_some()),
                "unexpected native host interaction: {reply}"
            );
        }
    }
    fn read(&mut self, id: &str) -> JsonValue {
        let reply = self.rpc("thread/read", json!({"threadId":id,"includeTurns":true}));
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"]["thread"].clone()
    }
    fn wait_idle(&mut self, id: &str) -> JsonValue {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let read = self.read(id);
            if read["status"]["type"] == "idle" {
                return read;
            }
            assert!(
                Instant::now() < deadline,
                "native turn did not complete: {read}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn respond_to_fixture(request: tiny_http::Request, number: usize) {
    let item = json!({"id":format!("msg-{number}"),"type":"message","role":"assistant",
        "content":[{"type":"output_text","text":format!("Preserved answer {number}."),"annotations":[]}]});
    let events = [
        json!({"type":"response.created","response":{"id":format!("response-{number}"),"status":"in_progress"}}),
        json!({"type":"response.output_item.done","item":item}),
        json!({"type":"response.completed","response":{"id":format!("response-{number}"),"status":"completed","output":[item],
            "usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}),
    ];
    let body = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect::<String>();
    request
        .respond(tiny_http::Response::from_string(body).with_header(
            tiny_http::Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
        ))
        .unwrap();
}

#[test]
#[ignore = "explicit isolated native archived-owner re-registration verification"]
fn original_enable_command_reregisters_an_archived_owner_without_resuming_the_caller() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let repo = root.join("reregistration-repo");
    let home = root.join("native-home");
    fs::create_dir(&repo).unwrap();
    fs::create_dir(&home).unwrap();
    init_repo(&InitRequest {
        root: repo.clone(),
        name: Some("reregistration-repo".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", server.server_addr());
    let (requested, requests) = mpsc::channel();
    let model = std::thread::spawn(move || {
        for number in 1..=2 {
            let request = server
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
                .expect("local model request missing");
            requested.send(number).unwrap();
            respond_to_fixture(request, number);
        }
    });
    let mut writer = Host::new(&home, &url);
    let enable = ["config", "set", "--agent-teams", "enabled", "--json"];
    let mut ids = Vec::new();
    let mut former_history = JsonValue::Null;
    for round in 0..2 {
        let start = writer.rpc(
            "thread/start",
            json!({"cwd":repo,"model":"fixture-model","ephemeral":false}),
        );
        let id = start["result"]["thread"]["id"]
            .as_str()
            .expect("native caller ID")
            .to_owned();
        let turn = writer.rpc("turn/start", json!({"threadId":id,"input":[{"type":"text","text":"Preserve this caller's existing context."}]}));
        assert!(turn.get("error").is_none(), "{turn}");
        assert_eq!(
            requests.recv_timeout(Duration::from_secs(15)).unwrap(),
            round + 1
        );
        let original = writer.wait_idle(&id);
        assert_eq!(original["turns"].as_array().unwrap().len(), 1);
        let offer = cli(&repo, &home, &id, &enable)["team_bootstrap"].clone();
        assert_eq!(offer["status"], "activation_required");
        assert_eq!(offer["session_id"], id);
        assert!(offer["role_instructions"]
            .as_str()
            .unwrap()
            .contains("Role MD:"));
        if round == 1 {
            assert_eq!(offer["previous_session_id"], ids[0]);
            assert_eq!(offer["replacement_reason"], "archived");
            let retry = cli(&repo, &home, &id, &enable)["team_bootstrap"].clone();
            assert_eq!(retry["activation_token"], offer["activation_token"]);
            assert!(retry["role_instructions"].is_null());
        }
        let named = writer.rpc(
            "thread/name/set",
            json!({"threadId":id,"name":offer["title"]}),
        );
        assert!(named.get("error").is_none(), "{named}");
        let ready = cli(
            &repo,
            &home,
            &id,
            &[
                "team",
                "session",
                "acknowledge",
                "--activation",
                offer["activation_token"].as_str().unwrap(),
                "--json",
            ],
        );
        assert_eq!(ready["status"], "ready");
        assert_eq!(writer.read(&id)["turns"], original["turns"]);
        if round == 0 {
            former_history = writer.read(&id)["turns"].clone();
            let archived = writer.rpc("thread/archive", json!({"threadId":id}));
            assert!(archived.get("error").is_none(), "{archived}");
        }
        ids.push(id);
    }
    model.join().unwrap();
    let contacts = cli(&repo, &home, &ids[1], &["team", "contacts", "--json"]);
    assert_eq!(contacts["bound_session_count"], 1);
    assert_eq!(contacts["members"][0]["session_id"], ids[1]);
    assert_eq!(contacts["members"][0]["role_reload_status"], "complete");
    assert_eq!(writer.read(&ids[0])["turns"], former_history);
    for (archived, expected) in [(true, &ids[0]), (false, &ids[1])] {
        let inventory = writer.rpc(
            "thread/list",
            json!({"limit":100,"archived":archived,"useStateDbOnly":true,
                "modelProviders":[],"sourceKinds":["cli","vscode","exec","appServer","subAgent","subAgentReview","subAgentCompact","subAgentThreadSpawn","subAgentOther","unknown"]}),
        );
        let rows = inventory["result"]["data"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "extra session: {inventory}");
        assert_eq!(rows[0]["id"], *expected);
    }
    assert_eq!(
        cli(&repo, &home, &ids[1], &enable)["team_bootstrap"]["action"],
        "reused_current"
    );
}

fn cli(repo: &Path, home: &Path, caller: &str, args: &[&str]) -> JsonValue {
    cli_with_status(repo, home, caller, args, true)
}
fn cli_with_status(
    repo: &Path,
    home: &Path,
    caller: &str,
    args: &[&str],
    success: bool,
) -> JsonValue {
    let output = Command::cargo_bin("ait-cli")
        .unwrap()
        .current_dir(repo)
        .env("CODEX_HOME", home)
        .env("CODEX_THREAD_ID", caller)
        .env("CODEX_SESSION_ID", caller)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "native activation CLI").unwrap()
}

#[test]
#[ignore = "explicit isolated native active-caller activation verification"]
fn enable_preserves_history_and_acknowledges_role_while_another_host_owns_the_active_writer() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let repo = root.join("caller-repo");
    let home = root.join("native-home");
    fs::create_dir(&repo).unwrap();
    fs::create_dir(&home).unwrap();
    init_repo(&InitRequest {
        root: repo.clone(),
        name: Some("logical-repository".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", server.server_addr());
    let (requested, requests) = mpsc::channel();
    let (release, hold) = mpsc::channel();
    let model = std::thread::spawn(move || {
        for number in 1..=2 {
            let request = server
                .recv_timeout(Duration::from_secs(20))
                .unwrap()
                .expect("local model request missing");
            requested.send(number).unwrap();
            if number == 2 {
                hold.recv_timeout(Duration::from_secs(20)).unwrap();
            }
            respond_to_fixture(request, number);
        }
    });
    let mut writer = Host::new(&home, &url);
    let start = writer.rpc(
        "thread/start",
        json!({"cwd":repo,"model":"fixture-model","ephemeral":false}),
    );
    let id = start["result"]["thread"]["id"]
        .as_str()
        .expect("native caller ID")
        .to_owned();
    let turn = writer.rpc(
        "turn/start",
        json!({"threadId":id,"input":[{"type":"text","text":"Existing caller question."}]}),
    );
    assert!(turn.get("error").is_none(), "{turn}");
    assert_eq!(requests.recv_timeout(Duration::from_secs(15)).unwrap(), 1);
    let original = writer.wait_idle(&id);
    assert_eq!(original["turns"].as_array().unwrap().len(), 1);
    assert!(original["turns"][0]
        .to_string()
        .contains("Preserved answer 1."));
    writer.rpc("turn/start",json!({"threadId":id,"input":[{"type":"text","text":"Keep this caller active during activation."}]}));
    assert_eq!(requests.recv_timeout(Duration::from_secs(15)).unwrap(), 2);
    assert_eq!(writer.read(&id)["status"]["type"], "active");
    let mut foreign = Host::new(&home, &url);
    let refused = foreign.rpc("thread/resume", json!({"threadId":id,"excludeTurns":true}));
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("active writer"),
        "{refused}"
    );
    assert!(!home
        .join("app-server-control/app-server-control.sock")
        .exists());
    let offer = cli(
        &repo,
        &home,
        &id,
        &["config", "set", "--agent-teams", "enabled", "--json"],
    )["team_bootstrap"]
        .clone();
    assert_eq!(offer["status"], "activation_required");
    assert_eq!(
        offer["title"],
        "［Newton］Project Manager·logical-repository"
    );
    let project = writer.rpc("project/read", json!({"projectId":offer["project_id"]}));
    assert_eq!(project["result"]["project"]["name"], "caller-repo");
    assert_eq!(
        project["result"]["project"]["roots"],
        json!([{"path":repo}])
    );
    assert!(offer["role_instructions"]
        .as_str()
        .unwrap()
        .contains(&fs::read_to_string(repo.join("docs/roles/0_Project Manager.md")).unwrap()));
    // Execute the app tool's native title operation on the existing caller host.
    assert_eq!(offer["app_actions"][0]["arguments"]["threadId"], id);
    writer.rpc(
        "thread/name/set",
        json!({"threadId":id,"name":"［Newton］Project Manager·caller-repo"}),
    );
    let incorrect = cli_with_status(
        &repo,
        &home,
        &id,
        &[
            "team",
            "session",
            "acknowledge",
            "--activation",
            offer["activation_token"].as_str().unwrap(),
            "--json",
        ],
        false,
    );
    assert_eq!(incorrect["status"], "pending");
    let named = writer.rpc(
        "thread/name/set",
        json!({"threadId":id,"name":offer["title"]}),
    );
    assert!(named.get("error").is_none(), "{named}");
    let acknowledged = cli(
        &repo,
        &home,
        &id,
        &[
            "team",
            "session",
            "acknowledge",
            "--activation",
            offer["activation_token"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(acknowledged["status"], "ready");
    assert_eq!(acknowledged["delivery_method"], "caller_acknowledged");
    let current = writer.read(&id);
    assert_eq!(current["status"]["type"], "active");
    assert_eq!(current["turns"][0], original["turns"][0]);
    assert_eq!(current["turns"].as_array().unwrap().len(), 2);
    assert_eq!(current["name"], offer["title"]);
    assert_eq!(current["projectId"], offer["project_id"]);
    let repeated = cli(
        &repo,
        &home,
        &id,
        &["config", "set", "--agent-teams", "enabled", "--json"],
    );
    assert_eq!(repeated["team_bootstrap"]["action"], "reused_current");
    assert!(repeated["team_bootstrap"]["role_instructions"].is_null());
    writer.rpc(
        "thread/name/set",
        json!({"threadId":id,"name":"［Newton］Project Manager·caller-repo"}),
    );
    let reconciled = cli(&repo, &home, &id, &["init", "--json"])["agent_team_bootstrap"].clone();
    assert_eq!(
        reconciled["title"],
        "［Newton］Project Manager·logical-repository"
    );
    assert_eq!(reconciled["session_id"], id);
    assert_eq!(reconciled["project_id"], offer["project_id"]);
    assert_eq!(
        reconciled["role_revision_index"],
        offer["role_revision_index"]
    );
    assert!(reconciled["role_instructions"].is_null());
    writer.rpc(
        "thread/name/set",
        json!({"threadId":id,"name":reconciled["title"]}),
    );
    let accepted = cli(
        &repo,
        &home,
        &id,
        &[
            "team",
            "session",
            "acknowledge",
            "--activation",
            reconciled["activation_token"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(accepted["status"], "ready");
    assert_eq!(accepted["title"], reconciled["title"]);
    assert_eq!(writer.read(&id)["turns"][0], original["turns"][0]);
    let contacts = cli(&repo, &home, &id, &["team", "contacts", "--json"]);
    assert_eq!(contacts["bound_session_count"], 1);
    assert_eq!(contacts["members"][0]["session_id"], id);
    assert_eq!(contacts["members"][0]["role_reload_status"], "complete");
    let inventory = writer.rpc("thread/list", json!({"limit":100,"useStateDbOnly":true}));
    assert_eq!(
        inventory["result"]["data"].as_array().unwrap().len(),
        1,
        "extra blank session: {inventory}"
    );
    release.send(()).unwrap();
    writer.wait_idle(&id);
    model.join().unwrap();
}
