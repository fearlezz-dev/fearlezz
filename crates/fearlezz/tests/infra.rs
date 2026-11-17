//! The infrastructure pieces: kustomize, helm, flux, terraform, opentofu, nix, yamllint and
//! sops. Each drives the real tool where devenv has it, and reads its own output.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// The checks Fearlezz knows, as its error for an unknown check lists them.
fn checks(repo: &Repo) -> String {
    let out = repo.fz(&["run", "no-such-check"]);
    assert!(!out.ok);
    out.stderr
        .split("checks are ")
        .nth(1)
        .unwrap_or_else(|| panic!("{}", out.stderr))
        .trim()
        .to_string()
}

/// No file Fearlezz stored holds `text`.
fn assert_nothing_stored_holds(repo: &Repo, text: &str) {
    let mut stack = vec![repo.path(".git/fearlezz")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(stored) = std::fs::read_to_string(&path) {
                assert!(!stored.contains(text), "{} holds it", path.display());
            }
        }
    }
}

/// A repository on a feature branch at the working stage, with `config` and `files`.
fn feature(config: &str, files: &[(&str, &str)]) -> Repo {
    let repo = Repo::new(config, files);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

// sops

const SOPS_YAML: &str =
    "creation_rules:\n  - path_regex: kubernetes/secrets/.*\\.yaml$\n    age: age1example\n";

/// What sops writes for a Secret: the values encrypted, and its metadata under `sops`.
const ENCRYPTED: &str = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: db\nstringData:\n  password: ENC[AES256_GCM,data:Zm9v,iv:YmFy,tag:YmF6,type:str]\nsops:\n  age:\n    - recipient: age1example\n  version: 3.13.3\n";

const PLAIN_VALUE: &str = "hunter2-in-plain-text";

fn plain() -> String {
    format!(
        "apiVersion: v1\nkind: Secret\nmetadata:\n  name: api\nstringData:\n  password: {PLAIN_VALUE}\n"
    )
}

fn sops_repo() -> Repo {
    feature(
        "use = [\"sops\"]\n",
        &[
            (".sops.yaml", SOPS_YAML),
            ("kubernetes/secrets/db.yaml", ENCRYPTED),
            // Matched by path_regex but never encrypted: from before the piece, untouched.
            (
                "kubernetes/secrets/kustomization.yaml",
                "resources:\n  - db.yaml\n",
            ),
        ],
    )
}

#[test]
fn the_sops_piece_gates_a_file_sops_yaml_names_that_is_not_encrypted() {
    let repo = sops_repo();
    assert_eq!(checks(&repo), "rules");

    repo.write(
        "kubernetes/secrets/db.yaml",
        &ENCRYPTED.replace("Zm9v", "YWJj"),
    );
    let out = repo.fz(&["run", "rules"]);
    assert!(
        out.ok,
        "an old plain file the change did not touch never blocks: {}",
        out.stdout
    );

    repo.write("kubernetes/secrets/api.yaml", &plain());
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("kubernetes/secrets/api.yaml"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("sops"), "{}", out.stdout);
    assert!(!out.stdout.contains(PLAIN_VALUE), "{}", out.stdout);
    assert_nothing_stored_holds(&repo, PLAIN_VALUE);
}

#[test]
fn a_file_sops_yaml_does_not_name_is_never_asked_to_be_encrypted() {
    let repo = sops_repo();
    repo.write("kubernetes/apps/config.yaml", &plain());
    let out = repo.fz(&["run", "rules"]);
    assert!(out.ok, "{}", out.stdout);
}

#[test]
fn a_matched_file_that_is_no_secret_is_skipped_in_the_rule() {
    let repo = sops_repo();
    repo.write(
        "fearlezz.toml",
        "use = [\"sops\"]\n\n[rule.sops]\nskip = [\"**/kustomization.yaml\"]\n",
    );
    repo.write(
        "kubernetes/secrets/kustomization.yaml",
        "resources:\n  - db.yaml\n  - api.yaml\n",
    );
    let out = repo.fz(&["run", "rules"]);
    assert!(out.ok, "{}", out.stdout);
}

#[test]
fn a_sops_yaml_in_a_folder_names_files_from_there() {
    let repo = feature(
        "use = [\"sops\"]\n",
        &[(
            "infra/.sops.yaml",
            "creation_rules:\n  - path_regex: ^secrets/.*$\n",
        )],
    );
    // Its path_regex is anchored at its own folder, so the root's secrets folder is not its.
    repo.write("secrets/notes.yaml", &plain());
    let out = repo.fz(&["run", "rules"]);
    assert!(out.ok, "{}", out.stdout);

    repo.write("infra/secrets/api.yaml", &plain());
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("infra/secrets/api.yaml"),
        "{}",
        out.stdout
    );
}

#[test]
fn json_and_dotenv_files_sops_encrypted_pass() {
    let repo = feature(
        "use = [\"sops\"]\n",
        &[(
            ".sops.yaml",
            "creation_rules:\n  - path_regex: \\.enc\\.(json|env)$\n",
        )],
    );
    repo.write(
        "app.enc.json",
        "{\n  \"token\": \"ENC[AES256_GCM,data:Zm9v,type:str]\",\n  \"sops\": {\"version\": \"3.13.3\"}\n}\n",
    );
    repo.write(
        "app.enc.env",
        "TOKEN=ENC[AES256_GCM,data:Zm9v,type:str]\nsops_version=3.13.3\n",
    );
    let out = repo.fz(&["run", "rules"]);
    assert!(out.ok, "{}", out.stdout);

    repo.write("app.enc.env", &format!("TOKEN={PLAIN_VALUE}\n"));
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("app.enc.env"), "{}", out.stdout);
    assert!(!out.stdout.contains(PLAIN_VALUE), "{}", out.stdout);
}

// kustomize

const DEPLOYMENT: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  selector:\n    matchLabels:\n      app: web\n  template:\n    metadata:\n      labels:\n        app: web\n    spec:\n      containers:\n        - name: web\n          image: nginx:1.27\n";

#[test]
fn the_kustomize_piece_builds_every_kustomization_not_inside_another() {
    let repo = feature(
        "use = [\"kustomize\"]\n",
        &[
            (
                "apps/base/kustomization.yaml",
                "resources:\n  - deployment.yaml\n",
            ),
            ("apps/base/deployment.yaml", DEPLOYMENT),
            (
                "apps/overlays/prod/kustomization.yaml",
                "resources:\n  - ../../base\n",
            ),
            ("infra/kustomization.yaml", "resources:\n  - addons\n"),
            ("infra/addons/kustomization.yaml", "resources: []\n"),
        ],
    );
    assert_eq!(
        checks(&repo),
        "kustomize/apps/base, kustomize/apps/overlays/prod, kustomize/infra"
    );
}

#[test]
fn a_kustomization_that_does_not_build_fails_before_kubeconform() {
    let repo = feature(
        "use = [\"kustomize\"]\n",
        &[
            (
                "apps/kustomization.yaml",
                "resources:\n  - deployment.yaml\n",
            ),
            ("apps/deployment.yaml", DEPLOYMENT),
        ],
    );
    repo.write("apps/kustomization.yaml", "resources:\n  - missing.yaml\n");
    let out = repo.fz(&["run", "kustomize/apps"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("missing.yaml"), "{}", out.stdout);
}

// helm

fn chart(template: &str) -> Vec<(String, String)> {
    vec![
        (
            "charts/web/Chart.yaml".into(),
            "apiVersion: v2\nname: web\nversion: 0.1.0\n".into(),
        ),
        ("charts/web/values.yaml".into(), "name: web\n".into()),
        (
            "charts/web/templates/configmap.yaml".into(),
            template.into(),
        ),
    ]
}

const CONFIGMAP: &str =
    "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {{ .Values.name }}\ndata:\n  a: b\n";

#[test]
fn the_helm_piece_lints_and_renders_every_chart() {
    let files = chart(CONFIGMAP);
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let repo = feature("use = [\"helm\"]\n", &files);
    assert_eq!(checks(&repo), "helm/charts/web");
    repo.write("charts/web/values.yaml", "name: site\n");
    let out = repo.fz(&["run", "helm/charts/web"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    repo.write(
        "charts/web/templates/configmap.yaml",
        &CONFIGMAP.replace(
            "{{ .Values.name }}",
            "{{ required \"a port\" .Values.port }}",
        ),
    );
    let out = repo.fz(&["run", "helm/charts/web"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("a port"), "{}", out.stdout);
}

// terraform and opentofu

const MODULE: &str = "variable \"name\" {\n  type = string\n}\n\noutput \"greeting\" {\n  value = \"hello ${var.name}\"\n}\n";

fn tofu_repo() -> Repo {
    feature(
        "use = [\"opentofu\"]\n",
        &[
            (".gitignore", ".terraform/\n"),
            ("infra/main.tf", MODULE),
            (
                "infra/.terraform.lock.hcl",
                "# This file is maintained automatically by \"tofu init\".\n",
            ),
        ],
    )
}

#[test]
fn the_opentofu_piece_validates_every_initialised_configuration() {
    let repo = tofu_repo();
    assert_eq!(checks(&repo), "tofu-fmt, tofu-validate/infra");
    repo.write("infra/main.tf", &MODULE.replace("hello", "hi"));
    let out = repo.fz(&["run", "tofu-validate/infra"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    repo.write("infra/main.tf", &MODULE.replace("var.name", "var.nope"));
    let out = repo.fz(&["run", "tofu-validate/infra"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("gate infra/main.tf:6  validate  Reference to undeclared input variable"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_validation_error_blocks_on_lines_the_change_did_not_touch() {
    let repo = tofu_repo();
    // The variable goes; the line that reads it stays as it was.
    repo.write(
        "infra/main.tf",
        &MODULE.replace("variable \"name\" {\n  type = string\n}\n\n", ""),
    );
    let out = repo.fz(&["run", "tofu-validate/infra"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("infra/main.tf:2"), "{}", out.stdout);
}

#[test]
fn unformatted_configuration_fails_tofu_fmt() {
    let repo = tofu_repo();
    repo.write(
        "infra/main.tf",
        &MODULE.replace("  type = string", "type=string"),
    );
    let out = repo.fz(&["run", "tofu-fmt"]);
    assert!(!out.ok, "{}", out.stdout);
}

#[test]
fn the_terraform_piece_has_the_same_checks_under_terraform() {
    let repo = Repo::new(
        "use = [\"terraform\"]\n",
        &[
            ("infra/main.tf", MODULE),
            ("infra/.terraform.lock.hcl", "\n"),
        ],
    );
    assert_eq!(checks(&repo), "terraform-fmt, terraform-validate/infra");
}

// nix

#[test]
fn the_nix_piece_checks_the_flake() {
    let repo = feature(
        "use = [\"nix\"]\n",
        &[("flake.nix", "{\n  outputs = _: { };\n}\n")],
    );
    assert_eq!(checks(&repo), "nix");
    repo.write("flake.nix", "{\n  outputs = _: { checks = { }; };\n}\n");
    repo.commit("checks");
    let out = repo.fz(&["run", "nix"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    repo.write("flake.nix", "{\n  outputs = _: { checks = 1 +; };\n}\n");
    let out = repo.fz(&["run", "nix"]);
    assert!(!out.ok, "{}", out.stdout);
}

// yamllint

#[test]
fn the_yamllint_piece_writes_its_config_and_gates_an_error_on_a_changed_line() {
    let repo = Repo::new(
        "use = [\"yamllint\"]\n",
        &[
            (".gitignore", "vendor/\n"),
            // As bad as the new one, but this change does not touch it.
            ("old.yaml", "a:   1\n"),
        ],
    );
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    let config = std::fs::read_to_string(repo.path(".yamllint.yaml")).unwrap();
    assert!(config.contains("ignore-from-file"), "{config}");
    repo.commit("sync");
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    // Git ignores it, so yamllint never reads it.
    repo.write("vendor/bad.yaml", "a:   1\n");

    repo.write("workflow.yaml", "on: push\nname: a very long line that is only flagged, never a gate, because manifests often have them\n");
    let out = repo.fz(&["run", "yamllint"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("flag workflow.yaml:2  line-length"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("vendor/bad.yaml"), "{}", out.stdout);

    repo.write("new.yaml", "a:   1\n");
    let out = repo.fz(&["run", "yamllint"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("gate new.yaml:1  colons"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("(1 more on lines this change did not touch)"),
        "{}",
        out.stdout
    );
}

// flux

#[test]
fn the_flux_piece_renders_tests_and_validates_in_one_check() {
    let repo = Repo::new("use = [\"flux\"]\n", &[]);
    assert_eq!(checks(&repo), "flate");
}
