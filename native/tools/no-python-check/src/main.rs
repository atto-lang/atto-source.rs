use std::{
    env, fs,
    path::Path,
    process::{self, Command},
};

fn forbidden_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let file = p.rsplit('/').next().unwrap_or(&p);
    p.ends_with(".py")
        || p.ends_with(".pyi")
        || p.ends_with(".pyw")
        || p.ends_with(".ipynb")
        || file == "pipfile"
        || file == "pipfile.lock"
        || file == "poetry.lock"
        || file == "pyproject.toml"
        || file == "uv.lock"
        || file == "setup.py"
        || file == "tox.ini"
        || file == ".python-version"
        || file == "pdm.lock"
        || (file.starts_with("requirements") && file.ends_with(".txt"))
}

fn is_execution_surface(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let file = p.rsplit('/').next().unwrap_or(&p);
    (p.starts_with(".github/workflows/") && (p.ends_with(".yaml") || p.ends_with(".yml")))
        || (p.starts_with(".github/actions/") && (p.ends_with(".yaml") || p.ends_with(".yml")))
        || file == "makefile"
        || file == "justfile"
        || p.ends_with(".sh")
        || p.ends_with(".bash")
        || p.ends_with(".zsh")
}

fn forbidden_execution_line(line: &str) -> bool {
    let l = line.trim().to_ascii_lowercase();
    // GitHub step labels describe the policy; they never launch a command.
    if l.starts_with('#') || l.starts_with("name:") || l.starts_with("- name:") {
        return false;
    }
    [
        "actions/setup-python@",
        "pip install",
        "pip3 install",
        "uv run ",
        "pytest ",
        "poetry run ",
        "pdm run ",
    ]
    .iter()
    .any(|needle| l.contains(needle))
        || python_command_word(&l)
}

// Reject executable interpreter command words, not descriptive substrings like
// "check-no-python". Versioned binaries and shell tab separators are included;
// the old trailing-ASCII-space check missed both, and a bare "python3" ran.
fn python_command_word(line: &str) -> bool {
    let bytes = line.as_bytes();
    for name in ["pythonw", "python", "pypy", "py"] {
        for (at, _) in line.match_indices(name) {
            if at > 0
                && !matches!(
                    bytes[at - 1],
                    b' ' | b'\t' | b':' | b'/' | b'\\' | b'(' | b';'
                        | b'&' | b'|' | b'=' | b'\'' | b'"' | b'\x60'
                )
            {
                continue;
            }
            let mut end = at + name.len();
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            // python3.12/pypy3.10, but not a file name like python3.md.
            while end + 1 < bytes.len()
                && bytes[end] == b'.'
                && bytes[end + 1].is_ascii_digit()
            {
                end += 1;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
            }
            // Include Windows .exe interpreter names without admitting .md paths.
            if line[end..].starts_with(".exe") {
                end += 4;
            }
            if end == bytes.len()
                || matches!(
                    bytes[end],
                    b' ' | b'\t' | b';' | b'&' | b'|' | b'(' | b')'
                        | b'<' | b'>' | b'$' | b'\'' | b'"' | b'\x60'
                )
            {
                return true;
            }
        }
    }
    false
}

fn python_shebang(body: &str) -> bool {
    let first = body
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    first.starts_with("#!")
        && (first.contains("/python")
            || first.contains("env python")
            || first.contains("env -s python")
            || first.contains("/pypy")
            || first.contains("env pypy"))
}

fn violations(path: &str, body: &str) -> Vec<String> {
    let mut messages = Vec::new();
    if forbidden_path(path) {
        messages
            .push("Python source, notebook, dependency manifest, or environment file".to_string());
    }
    if python_shebang(body) {
        messages.push("Python/PyPy shebang".to_string());
    }
    if is_execution_surface(path) {
        for (index, line) in body.lines().enumerate() {
            if forbidden_execution_line(line) {
                messages.push(format!("Python tooling invocation at line {}", index + 1));
            }
        }
    }
    messages
}

fn main() {
    let root = env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let output = Command::new("git")
        .args(["-C", &root, "ls-files", "-z"])
        .output()
        .expect("git must be installed");
    if !output.status.success() {
        eprintln!("git ls-files failed");
        process::exit(2);
    }

    let mut found = 0usize;
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|v| !v.is_empty())
    {
        let name = String::from_utf8_lossy(raw);
        let file = Path::new(&root).join(name.as_ref());

        // Path-only checks still apply even when the content is binary or unreadable.
        let body = fs::read_to_string(&file).unwrap_or_default();
        for problem in violations(&name, &body) {
            eprintln!("{name}: {problem}");
            found += 1;
        }
    }

    if found > 0 {
        eprintln!("{found} Python policy violation(s)");
        process::exit(1);
    }
    println!("Rust-only tooling policy check passed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_python_assets_and_manifests() {
        for path in [
            "tool/doit.py",
            "tool/doit.pyw",
            "types/x.pyi",
            "tests/a.ipynb",
            "pyproject.toml",
            "requirements-dev.txt",
            "Pipfile.lock",
            ".python-version",
            "tox.ini",
        ] {
            assert!(forbidden_path(path), "must reject {path}");
        }
        assert!(!forbidden_path("README.md"));
        assert!(!forbidden_path("src/main.rs"));
    }

    #[test]
    fn recognizes_execution_surfaces() {
        assert!(is_execution_surface(".github/workflows/ci.yml"));
        assert!(is_execution_surface(".github/actions/check/action.yaml"));
        assert!(is_execution_surface("scripts/check.sh"));
        assert!(is_execution_surface("Makefile"));
        assert!(is_execution_surface("justfile"));
        assert!(!is_execution_surface("docs/policy.md"));
    }

    #[test]
    fn blocks_python_ci_invocations() {
        assert!(forbidden_execution_line("- uses: actions/setup-python@v5"));
        assert!(forbidden_execution_line("- run: python3 -m pytest"));
        assert!(forbidden_execution_line("pip install -r requirements.txt"));
        assert!(forbidden_execution_line("uv run tool.py"));
        assert!(!forbidden_execution_line("- run: cargo test"));
        assert!(!forbidden_execution_line("- name: Reject Python files and execution"));
        assert!(!forbidden_execution_line("name: Prohibit Python"));
        assert!(!forbidden_execution_line("/tmp/check-no-python ."));
        assert!(!forbidden_execution_line("run: cargo run --manifest-path tools/no-python-check/Cargo.toml -- ."));
        assert!(forbidden_execution_line("run: /tmp/check-no-python . && python3 tests/run.py"));
        assert!(forbidden_execution_line("/usr/bin/python3 script.py"));
    }

    #[test]
    fn rejects_interpreter_command_boundary_evasions() {
        // Earlier scanner recognized only literal "python3 " and could
        // admit tab-separated, versioned, or bare interpreter commands.
        for line in [
            "- run: python3\t-m pytest",
            "run: /usr/bin/python3.12 -I -m unittest",
            "run: /usr/bin/pypy3.10 -m pytest",
            "run: /usr/bin/env -S python3 -I",
            "python3",
            "python3\t-c 'print(1)'",
            "run: python3$ARGS",
            "run: exec python",
            "run: python3.12",
        ] {
            assert!(forbidden_execution_line(line), "missed executable: {line}");
        }
        for line in [
            "- run: cargo test --bin check-no-python3",
            "- name: Use Python3 as test reference",
            "/tmp/check-no-python3 .",
            "run: echo status=python3-result",
            "run: echo docs/python3.md",
        ] {
            assert!(!forbidden_execution_line(line), "false positive: {line}");
        }
    }

    #[test]
    fn rejects_quoted_and_windows_interpreter_invocations() {
        for line in [
            "run: bash -lc 'python3 -m pytest'",
            "run: sh -c \"python3 -c 'print(1)'\"",
            r#"run: & "C:\Python312\python.exe" -m pytest"#,
            r#"run: & "C:\Python312\pythonw.exe" script.py"#,
            "run: python3.12.exe -m pytest",
            "run: pypy3.10.exe -m pytest",
            "run: py -3 -m pytest",
            "run: py.exe -3 -m pytest",
            "run: \u{60}python3\u{60}",
            "run: python3.exe",
        ] {
            assert!(forbidden_execution_line(line), "missed interpreter: {line}");
        }
        for line in [
            "run: cargo test --bin check-no-python.exe",
            "run: echo docs/python3.md",
            "run: echo docs/pyproject.toml",
            "run: echo 'no-python-control'",
            "- name: Check py.exe launcher policy",
        ] {
            assert!(!forbidden_execution_line(line), "false positive: {line}");
        }
    }

    #[test]
    fn blocks_extensionless_python_scripts() {
        assert!(python_shebang("#!/usr/bin/env python3\nprint('x')"));
        assert!(python_shebang("#!/usr/bin/python\nprint('x')"));
        assert!(python_shebang("#!/usr/bin/env pypy3\nprint('x')"));
        assert!(!python_shebang("#!/usr/bin/env bash\necho ok"));
    }

    #[test]
    fn prose_is_not_treated_as_execution() {
        let body = "Python is prohibited; replace Python tooling with Rust.";
        assert!(violations("README.md", body).is_empty());
    }
}
