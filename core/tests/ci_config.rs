//! Разбор .gitlab-ci.yml и GitHub Actions без YAML-библиотеки.
mod common;

use pult_core::ci_config::{parse_github, parse_gitlab};
use pult_core::{CiConfig, CiJobCfg};

fn job<'a>(c: &'a CiConfig, name: &str) -> &'a CiJobCfg {
    c.stages.iter().flat_map(|s| &s.jobs).find(|j| j.name == name).unwrap_or_else(|| panic!("нет джобы «{name}»: {:?}", names(c)))
}

fn names(c: &CiConfig) -> Vec<String> {
    c.stages.iter().flat_map(|s| s.jobs.iter().map(|j| j.name.clone())).collect()
}

const GITLAB: &str = r#"stages:
  - build
  - test
  - deploy

variables:
  NODE_ENV: test

.base:
  stage: test
  image: node:20

# Сборка сайта
build:
  stage: build
  script:
    - pnpm build

test:api:
  extends: .base
  script:
    - pnpm test:api

build job:
  stage: build
  script:
    - make

deploy:
  stage: deploy
  script:
    - ./deploy.sh
  when: manual
  allow_failure: true
"#;

#[test]
fn gitlab_job_names_with_colon_and_space() {
    let c = parse_gitlab(GITLAB, &common::kb());
    assert_eq!(c.kind, "gitlab");
    let n = names(&c);
    assert!(n.contains(&"test:api".to_string()), "{n:?}");
    assert!(n.contains(&"build job".to_string()), "{n:?}");
    assert_eq!(c.job_count, 4, "скрытый шаблон .base не джоба: {n:?}");
    assert!(!n.iter().any(|x| x.starts_with('.')));
}

#[test]
fn gitlab_job_with_space_does_not_swallow_previous() {
    let c = parse_gitlab(GITLAB, &common::kb());
    // «build job:» раньше читался как продолжение блока «test:api», и тот терял своё
    let t = job(&c, "test:api");
    assert_eq!(t.stage, "test");
    assert_eq!(job(&c, "build job").stage, "build");
    assert_eq!(job(&c, "build").line, 14);
    assert_eq!(job(&c, "build job").line, 24);
}

#[test]
fn gitlab_extends_takes_stage_from_hidden_template() {
    let c = parse_gitlab(GITLAB, &common::kb());
    assert_eq!(job(&c, "test:api").stage, "test");
}

#[test]
fn gitlab_stages_list_keeps_order_and_comment_becomes_desc() {
    let c = parse_gitlab(GITLAB, &common::kb());
    let st: Vec<&str> = c.stages.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(st, ["build", "test", "deploy"]);
    assert_eq!(job(&c, "build").desc, "Сборка сайта");
    assert_eq!(c.stages[0].sid, "build");
    assert_eq!(c.stages[1].sid, "check");
    assert_eq!(c.stages[2].sid, "release");
}

#[test]
fn gitlab_manual_and_allow_failure() {
    let c = parse_gitlab(GITLAB, &common::kb());
    let d = job(&c, "deploy");
    assert!(d.manual);
    assert!(d.allow_failure);
    assert!(!job(&c, "build").manual);
}

#[test]
fn gitlab_stages_in_brackets_and_needs() {
    let text = "stages: [lint, \"build\", 'ship']\n\nlint:\n  stage: lint\n  script: [eslint .]\n\npack:\n  stage: build\n  needs: [lint]\n  script:\n    - make\n\nrelease:\n  stage: ship\n  script:\n    - ./ship.sh\n  rules:\n    - if: $CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH\n";
    let c = parse_gitlab(text, &common::kb());
    let st: Vec<&str> = c.stages.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(st, ["lint", "build", "ship"]);
    assert_eq!(job(&c, "pack").needs, vec!["lint".to_string()]);
    assert!(job(&c, "release").only_main);
    assert!(!job(&c, "pack").only_main);
}

#[test]
fn gitlab_without_stages_uses_default_order_and_test_stage() {
    let text = "image: node:20\n\nunit:\n  script:\n    - npm test\n";
    let c = parse_gitlab(text, &common::kb());
    assert_eq!(job(&c, "unit").stage, "test", "джоба без stage идёт в test, как в самом GitLab");
    assert_eq!(c.job_count, 1, "image: не джоба");
}

#[test]
fn gitlab_local_includes() {
    let text = "include:\n  - local: ci/build.yml\n  - local: 'ci/test.yml'\n";
    let c = parse_gitlab(text, &common::kb());
    assert_eq!(c.includes, vec!["ci/build.yml".to_string(), "ci/test.yml".to_string()]);
}

#[test]
fn github_name_inline_on_and_needs() {
    let text = "name: CI\non: [push, pull_request]\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: make\n  test:\n    name: Unit tests\n    needs: [build]\n    runs-on: ubuntu-latest\n";
    let c = parse_github(text, ".github/workflows/ci.yml", &common::kb());
    assert_eq!(c.kind, "github");
    assert_eq!(c.name, "CI");
    assert_eq!(c.triggers, vec!["push".to_string(), "pull_request".to_string()]);
    assert_eq!(c.job_count, 2);
    assert_eq!(c.jobs[0].id, "build");
    assert_eq!(c.jobs[0].name, "build");
    assert_eq!(c.jobs[1].id, "test");
    assert_eq!(c.jobs[1].name, "Unit tests");
    assert_eq!(c.jobs[1].needs, vec!["build".to_string()]);
    assert_eq!(c.jobs[1].line, 8);
    assert_eq!(c.jobs[1].sid, "check");
}

#[test]
fn github_on_block_and_needs_scalar() {
    let text = "on:\n  push:\n    branches: [main]\n  workflow_dispatch:\njobs:\n  deploy:\n    needs: build\n    runs-on: ubuntu-latest\n";
    let c = parse_github(text, ".github/workflows/release.yml", &common::kb());
    assert_eq!(c.name, "release.yml", "без name: - имя файла");
    assert_eq!(c.triggers, vec!["push".to_string(), "workflow_dispatch".to_string()]);
    assert_eq!(c.jobs.len(), 1);
    assert_eq!(c.jobs[0].needs, vec!["build".to_string()]);
    assert_eq!(c.jobs[0].sid, "release");
}

#[test]
fn gitlab_needs_as_block_list() {
    let text = "build:\n  stage: build\n  script: [make]\n\ne2e:\n  stage: test\n  needs:\n    - build\n    - job: lint\n      artifacts: false\n    - project: group/other\n      job: x\n  script:\n    - make e2e\n";
    let c = parse_gitlab(text, &common::kb());
    assert_eq!(job(&c, "e2e").needs, vec!["build".to_string(), "lint".to_string()], "чужой проект в needs - не джоба этого файла");
}

#[test]
fn github_needs_as_block_list() {
    let text = "jobs:\n  a:\n    runs-on: x\n  b:\n    needs:\n      - a\n      - \"c\"\n    runs-on: x\n";
    let c = parse_github(text, "w.yml", &common::kb());
    assert_eq!(c.jobs[1].needs, vec!["a".to_string(), "c".to_string()]);
    assert!(c.jobs[0].needs.is_empty());
}

#[test]
fn github_four_space_indent_and_comments() {
    // отступ в 4 пробела и комментарии в конце строк - частый вид настоящих workflow
    let text = "name: Build # главный\non: # когда\n    push:\n        branches: [main]\n    pull_request:\njobs: # джобы\n    lint:\n        name: Lint code\n        runs-on: ubuntu-latest\n        steps:\n            - run: npm run lint\n    test:\n        needs: [lint] # сначала линтер\n        runs-on: ubuntu-latest\n    deploy:\n        needs:\n            - test\n            - lint\n        runs-on: ubuntu-latest\n";
    let c = parse_github(text, ".github/workflows/ci.yml", &common::kb());
    assert_eq!(c.name, "Build");
    assert_eq!(c.triggers, vec!["push".to_string(), "pull_request".to_string()]);
    let ids: Vec<&str> = c.jobs.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, ["lint", "test", "deploy"]);
    assert_eq!(c.jobs[0].name, "Lint code");
    assert_eq!(c.jobs[1].needs, vec!["lint".to_string()]);
    assert_eq!(c.jobs[2].needs, vec!["test".to_string(), "lint".to_string()]);
}

#[test]
fn gitlab_stage_with_comment_and_commented_stages_list() {
    let text = "stages:\n  - build # сборка\n  - test\n\nweb:\n  stage: build # сайт\n  extends: .base # общий\n  script: [make]\n\n.base:\n  image: node:22\n";
    let c = parse_gitlab(text, &common::kb());
    let st: Vec<&str> = c.stages.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(st, ["build", "test"]);
    assert_eq!(job(&c, "web").stage, "build", "комментарий после этапа не прячет этап");
}

#[test]
fn gitlab_yaml_anchor_merge_is_a_job_with_template_stage() {
    let text = "stages: [build, deploy]\n\n.deploy-tpl: &deploy\n  stage: deploy\n  script:\n    - ./deploy.sh\n\nprod:\n  <<: *deploy\n  environment: production\n";
    let c = parse_gitlab(text, &common::kb());
    assert_eq!(c.job_count, 1, "{:?}", names(&c));
    assert_eq!(job(&c, "prod").stage, "deploy");
}
