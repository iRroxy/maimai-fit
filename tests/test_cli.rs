use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use parking_lot::Mutex;
use tempfile::tempdir;

const BIN: &str = env!("CARGO_BIN_EXE_maimai");
const DEMO_KEEP: &str = "示例鸡肉主食（一份）";

// A concurrently forked child can retain a copied executable's writable descriptor until exec.
static EXECUTABLE_IO: Mutex<()> = Mutex::new(());

fn copy_binary(destination: &Path) {
    let _guard = EXECUTABLE_IO.lock();
    fs::copy(BIN, destination).unwrap();
}

fn run(binary: &Path, cwd: &Path, args: &[&str]) -> Output {
    let _guard = EXECUTABLE_IO.lock();
    Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .env_remove("MCD_MCP_TOKEN")
        .stdin(Stdio::null())
        .output()
        .expect("run real CLI")
}

fn cli(cwd: &Path, args: &[&str]) -> Output {
    run(Path::new(BIN), cwd, args)
}

fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn result(output: &Output) -> Value {
    assert_exit(output, 0);
    serde_json::from_slice(&output.stdout).expect("CLI JSON result")
}

fn assert_input_error(output: &Output, message: &str) {
    assert_exit(output, 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(message), "{stderr}");
    assert!(!stderr.contains("MCD_MCP_TOKEN"), "{stderr}");
    assert!(!stderr.contains("隐藏输入"), "{stderr}");
}

#[test]
fn standalone_demo_embeds_catalog_and_writes_bare_html_without_overwriting() {
    let workspace = tempdir().unwrap();
    let binary = workspace.path().join("standalone-maimai");
    copy_binary(&binary);
    let cwd = workspace.path().join("caller");
    fs::create_dir(&cwd).unwrap();

    let output = run(&binary, &cwd, &["demo", "--json", "--html", "report.html"]);
    let data = result(&output);
    assert_eq!(data["source"]["kind"], "demo");
    assert_eq!(data["constraints"]["keep"], json!([DEMO_KEEP]));
    let report = fs::read_to_string(cwd.join("report.html")).unwrap();
    assert!(report.contains(DEMO_KEEP));
    assert!(!cwd.join("examples").exists());

    let refused = run(&binary, &cwd, &["demo", "--html", "report.html"]);
    assert_input_error(&refused, "不覆盖已有路径");
    assert_eq!(fs::read_to_string(cwd.join("report.html")).unwrap(), report);
}

#[test]
fn installed_rust_skill_keeps_relative_catalog_and_html_in_caller_cwd() {
    let workspace = tempdir().unwrap();
    let installed = workspace.path().join("nested").join("installed-skill");
    let caller = workspace.path().join("caller");
    fs::create_dir(&caller).unwrap();
    let running_binary = workspace.path().join(format!("standalone{}", std::env::consts::EXE_SUFFIX));
    copy_binary(&running_binary);
    // A decoy release tree must never be used instead of the running executable.
    fs::create_dir_all(caller.join("target/release")).unwrap();
    let decoy = caller.join("target/release").join(format!("maimai{}", std::env::consts::EXE_SUFFIX));
    fs::write(&decoy, "not the running binary").unwrap();
    let output = run(&running_binary, &caller, &["install", "--to", installed.to_str().unwrap()]);
    assert_exit(&output, 0);
    let scripts = installed.join("scripts");
    let runner = scripts.join(format!("maimai{}", std::env::consts::EXE_SUFFIX));
    assert!(!caller.join("skills").exists());
    fs::remove_file(&running_binary).unwrap();
    let invoke = |args: &[&str]| run(&runner, &caller, args);

    let refused = invoke(&["install", "--to", installed.to_str().unwrap()]);
    assert_input_error(&refused, "安装目标已存在");
    assert_eq!(fs::read(&decoy).unwrap(), b"not the running binary");

    // Reinstallation is also standalone: its Skill and executable are embedded/self-copied.
    let second = caller.join("second-skill");
    assert_exit(&invoke(&["install", "--to", "second-skill"]), 0);
    let reinstalled = second.join("scripts").join(format!("maimai{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(result(&run(&reinstalled, &caller, &["demo", "--json"]))["plans"][0]["totals"]["kcal"], "540");

    let demo = invoke(&["demo", "--json", "--html", "demo.html"]);
    assert_eq!(result(&demo)["source"]["kind"], "demo");
    assert!(caller.join("demo.html").is_file());

    let catalog = json!({
        "schema_version": 1,
        "source": {"kind": "import"},
        "foods": [{"name": "Caller food", "kcal": "500", "protein_g": "35"}]
    });
    fs::write(caller.join("catalog.json"), catalog.to_string()).unwrap();
    let planned = invoke(&[
        "plan", "--catalog", "catalog.json", "--keep", "Caller food",
        "--json", "--html", "plan.html",
    ]);
    assert_eq!(result(&planned)["source"]["kind"], "import");
    assert!(caller.join("plan.html").is_file());
    assert!(!scripts.join("plan.html").exists());
    assert!(!scripts.join("demo.html").exists());

    let no_solution = invoke(&[
        "plan", "--catalog", "catalog.json", "--keep", "Caller food",
        "--min-protein", "100", "--json",
    ]);
    assert_exit(&no_solution, 2);
    let data: Value = serde_json::from_slice(&no_solution.stdout).unwrap();
    assert_eq!(data["status"], "infeasible");
}

#[test]
fn install_refuses_existing_files_and_invalid_parent_without_touching_them() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("existing"), "preserve").unwrap();
    assert_input_error(&cli(directory.path(), &["install", "--to", "existing"]), "安装目标已存在");
    assert_exit(&cli(directory.path(), &["install", "--to", "existing/child/skill"]), 1);
    assert_eq!(fs::read_to_string(directory.path().join("existing")).unwrap(), "preserve");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn install_refuses_existing_and_dangling_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().unwrap();
    fs::create_dir(directory.path().join("real")).unwrap();
    for (name, target) in [("existing", "real"), ("dangling", "absent")] {
        symlink(target, directory.path().join(name)).unwrap();
        assert_input_error(&cli(directory.path(), &["install", "--to", name]), "安装目标已存在");
        assert_eq!(fs::read_link(directory.path().join(name)).unwrap(), Path::new(target));
    }
    assert_eq!(fs::read_dir(directory.path().join("real")).unwrap().count(), 0);
    assert!(!directory.path().join("absent").exists());
}

#[test]
fn fetch_rejects_local_paths_and_json_before_credentials() {
    let workspace = tempdir().unwrap();
    let cwd = workspace.path();
    fs::create_dir(cwd.join("nested")).unwrap();
    fs::write(cwd.join("existing.json"), "do not overwrite").unwrap();

    for alias in [
        cwd.join("catalog.json"),
        cwd.join("nested").join("..").join("catalog.json"),
    ] {
        let output = cli(cwd, &[
            "fetch", "--out", "catalog.json", "--raw-out", alias.to_str().unwrap(),
            "--ask-token",
        ]);
        assert_input_error(&output, "不能使用同一输出路径");
    }
    let same_relative = cli(cwd, &[
        "fetch", "--out", "catalog.json", "--raw-out", "./catalog.json", "--ask-token",
    ]);
    assert_input_error(&same_relative, "不能使用同一输出路径");

    for arguments in ["{", "{} trailing", "[]", "null", "true", "{\"x\":1,\"x\":2}", "{\"x\":NaN}", "{\"x\":Infinity}"] {
        let output = cli(cwd, &[
            "fetch", "--out", "catalog.json", "--arguments", arguments, "--ask-token",
        ]);
        assert_input_error(&output, "--arguments 必须是");
    }
    let invalid_parent = cli(cwd, &[
        "fetch", "--out", "missing/catalog.json", "--ask-token",
    ]);
    assert_input_error(&invalid_parent, "输出目录不存在");
    let existing = cli(cwd, &[
        "fetch", "--out", "existing.json", "--ask-token",
    ]);
    assert_input_error(&existing, "不覆盖已有路径");
    let existing_raw = cli(cwd, &[
        "fetch", "--out", "catalog.json", "--raw-out", "existing.json", "--ask-token",
    ]);
    assert_input_error(&existing_raw, "不覆盖已有路径");
    assert_eq!(fs::read_to_string(cwd.join("existing.json")).unwrap(), "do not overwrite");
    assert!(!cwd.join("catalog.json").exists());

    // A valid bare filename reaches credential handling without making a request.
    let valid = cli(cwd, &["fetch", "--out", "catalog.json", "--ask-token"]);
    assert_exit(&valid, 1);
    assert!(String::from_utf8_lossy(&valid.stderr).contains("--ask-token 只能"));
}

#[cfg(unix)]
#[test]
fn fetch_rejects_symlink_parent_alias_before_credentials() {
    use std::os::unix::fs::symlink;

    let workspace = tempdir().unwrap();
    let cwd = workspace.path();
    fs::create_dir(cwd.join("real")).unwrap();
    symlink(cwd.join("real"), cwd.join("alias")).unwrap();
    let output = cli(cwd, &[
        "fetch", "--out", "real/catalog.json", "--raw-out", "alias/catalog.json",
        "--ask-token",
    ]);
    assert_input_error(&output, "不能使用同一输出路径");
    assert!(!cwd.join("real/catalog.json").exists());
}

#[test]
fn exclude_preserves_comma_substrings_and_repeated_original_arguments() {
    let workspace = tempdir().unwrap();
    let cwd = workspace.path();
    let baseline = result(&cli(cwd, &["demo", "--json"]));
    let comma = result(&cli(cwd, &[
        "demo", "--json", "--exclude", " 不存在,示例 ",
    ]));
    assert_eq!(comma["constraints"]["exclude"], json!([" 不存在,示例 "]));
    assert_eq!(comma["plans"], baseline["plans"]);
    assert_eq!(comma["search"], baseline["search"]);

    let repeated = result(&cli(cwd, &[
        "demo", "--json", "--exclude", " 奶饮 ", "--exclude", "甜点",
        "--exclude", " 奶饮 ",
    ]));
    assert_eq!(repeated["constraints"]["exclude"], json!([" 奶饮 ", "甜点", " 奶饮 "]));
    for plan in repeated["plans"].as_array().unwrap() {
        for food in plan["foods"].as_array().unwrap() {
            let name = food["name"].as_str().unwrap();
            assert!(!name.contains("奶饮") && !name.contains("甜点"));
        }
    }
    for empty in ["", " \t "] {
        assert_input_error(
            &cli(cwd, &["demo", "--exclude", empty]),
            "排除关键词不能为空",
        );
    }
    assert_input_error(
        &cli(cwd, &["demo", "--exclude", " 示例鸡肉 "]),
        "保留与排除条件冲突",
    );
}
