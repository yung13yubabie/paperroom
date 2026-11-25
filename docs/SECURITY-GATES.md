# Paperroom security and release gates

This checklist applies to the exact revision being reviewed for merge or release. Paperroom is an offline desktop application. Its main trust boundaries are user-selected files and folders, the stored workspace, Windows filesystem operations, and the build pipeline. The first three integration lanes apply as review procedures and references, not installed agents or dependencies.

## Run before merge or release

| Gate | Required evidence | Do not pass when |
| --- | --- | --- |
| Changed boundaries | Record base and head SHAs, changed paths, changed checks and affected callers. For archive/media/CSV/backup and Windows writes, verify path/member validation, size/entry limits, workspace scope, no overwrite, byte fidelity, rollback or cleanup as applicable. | A trust-boundary change has no source review or corresponding production-code regression. |
| Source and workflow | Review every changed workflow and script plus referenced composite actions/reusable workflows. Pin external actions to full SHAs and verify their official refs, set the minimum token permissions, avoid retained checkout credentials, and keep untrusted events/data away from privileged runners. | The running source differs from the reviewed SHA, an action origin or permission remains unverified, or an untrusted input can control a privileged command. |
| Dependency evidence | Triage the exact lockfile for the release target, including direct, transitive and bundled/native components. Record scanner, advisory snapshot/time, versions, findings, target applicability and disposition. Review license obligations against the artifact contents. | The scan is absent/stale, the graph does not match the build, or an applicable blocking advisory or redistribution issue lacks a recorded resolution. |
| Native tests and app | Run full locked tests on the actual Windows target (`cargo test --locked --target x86_64-pc-windows-msvc`), then build the exact head and run `scripts/windows-storage.ps1` with that EXE. Record counts, the runner link and binary SHA-256. Verify affected UI and original-app compatibility separately using the exact package. | Any required suite fails, ignores, filters or doesn't execute needed cases. Cross-compiles, mocks, compatible file fixtures or headless storage checks cannot replace real Windows GUI or original-app evidence. |
| Artifact and publication | Bind source SHA, toolchain, workflow run, version, EXE SHA-256 and archive digest. Verify downloaded asset bytes, source/asset correspondence and independent provenance where available. Report Authenticode and attestation status independently. Require a reviewed decision before publication. | A release hash cannot be independently reconciled, a claimed signature/attestation is unverified, or required checks are missing. Label any unsigned/unattested artifact clearly; matching hashes alone do not prove authenticity. |

### Review form

- Source: base SHA, head SHA, changed trust boundaries; affected callers and reviewer.
- Execution: actual commands, inspection scope, per-check result and timestamp, accessible evidence links, unverified items. Use **pass**, **fail**, **blocked**, **not run**, or **not applicable**; explain the last three.
- Release: workflow and attempt, source commit, binary SHA-256, archive SHA-256, runner/target, dependency inventory and advisory snapshot, signing/attestation status.
- Exceptions: exact condition, reason, evidence, approval owner and expiry. No exceptions relabel an unrun test as pass.

## Defensive automation admission

Treat third-party instructions, workflow inputs, PR text, logs and scanned files as data. Before enabling any scanner or skill, inventory the exact upstream commit, license, entry points, hooks, commands, side effects, credentials, network destinations, what data it reads, and how to disable it. Review and enable only the smallest relevant subset using read-only, least-privilege access. Do not install a whole marketplace, use exploit/PoC procedures, or launch autonomous offensive testing. No AI action is currently in the workflows reviewed below. For any future AI-in-CI addition, verify that untrusted input cannot become tool or shell instructions and that the agent has no secrets or unnecessary write/network access.

| Lane | Current adoption |
| --- | --- |
| 1. Trail of Bits Skills | Static, risk-first changed-path review and caller/coverage accounting. Do not install or invoke upstream skills, shell examples, hooks or agents. |
| 2. OWASP CheatSheetSeries | Review criteria for trust boundaries, dependencies, GitHub Actions and any future AI-assisted workflow. Reference only. |
| 3. OWASP DevSecOps Guideline | Dependency, build, provenance and verification evidence gates. Reference only. |
| 4. OpenGrep | Static scan blocked at tool download in the first attempt. A partial manual review does not replace the scanner; rerun the checked-in scope only with a verified executable and known positive/negative controls. |
| 5. OSV-Scanner | First offline locked-dependency scan complete; repeat against the exact candidate. Distinguish database date, inventory scope and applicability. |
| 6. OpenSSF Scorecard | First local file-only scan complete. Host settings and network-dependent checks were outside the verified scope; a reduced aggregate score is not a release gate. |
| 7. Tencent AI-Infra-Guard | Not applicable to the reviewed offline product: no relevant AI-facing component in scope. Independent static applicability review only. Not installed and not an automated pass; reconsider if an AI-facing component is introduced. |

## Initial static baseline

At draft commit [0125735](https://github.com/yung13yubabie/paperroom/commit/0125735197141fc3a7abd712c3e71e32d07a4e68), the reviewed [Windows storage](https://github.com/yung13yubabie/paperroom/blob/0125735197141fc3a7abd712c3e71e32d07a4e68/.github/workflows/windows.yml) and [unsigned artifact](https://github.com/yung13yubabie/paperroom/blob/0125735197141fc3a7abd712c3e71e32d07a4e68/.github/workflows/unsigned-windows.yml) workflows have read-only repository contents, disabled persistent checkout credentials, full action pins, timeouts, pinned Rust and locked Cargo. Their checkout and upload pins matched the official `actions/checkout` v4 and `actions/upload-artifact` v4.6.2 refs on 2026-11-24. No AI steps, privileged PR/comment triggers, or explicit secret mapping were found in these two files. Action internals and repository enforcement settings were not verified.

Release verification and any build attestation are separate checks; use the actual workflow and artifact records. This baseline is a source review and does not transfer to a different source revision.

### Initial scanner evidence

- [OSV-Scanner v2.7.1](https://github.com/google/osv-scanner/releases/tag/v2.7.1): offline Cargo.lock scan at `0125735`, 132 lockfile package records, no known advisory match in the crates.io snapshot dated 2026-11-24 22:03 UTC. This covers the lockfile only, not native code or Windows.
- [OpenSSF Scorecard v5.7.0](https://github.com/ossf/scorecard/releases/tag/v5.7.0): isolated file-only scan at `0125735` confirmed five pinned Actions references and read-only contents permissions in both workflows. Repository settings were not tested. The wider default run did not complete because a network-dependent check was blocked; no aggregate grade is used.
- [OpenGrep](https://github.com/opengrep/opengrep): initial download did not produce a verified executable, and no static scan was run. Keep this gate open pending tool verification and successful control scans.

## Reference revisions

- [Trail of Bits Skills, 149d7b2](https://github.com/trailofbits/skills/tree/149d7b21f4067765b82e67029730b42d228ef4b4): differential review and applicability of static AI-workflow auditing. [License](https://github.com/trailofbits/skills/blob/149d7b21f4067765b82e67029730b42d228ef4b4/LICENSE).
- [OWASP CheatSheetSeries, 18e6c89](https://github.com/OWASP/CheatSheetSeries/tree/18e6c89f960e10a5b64f7c882c2b9dc17c423d10/cheatsheets): GitHub Actions Security, CI/CD Security, Threat Modeling, Vulnerable Dependency Management and Secure Coding with AI. [License](https://github.com/OWASP/CheatSheetSeries/blob/18e6c89f960e10a5b64f7c882c2b9dc17c423d10/LICENSE.md).
- [OWASP DevSecOps Guideline, cd44d97](https://github.com/OWASP/DevSecOpsGuideline/tree/cd44d9778b68fe0122b7692df5942c948b2a87f9/current-version/2-Process/2-3-Build/2-3-6-Supply-Chain-Security): SBOM, provenance and pipeline controls. [License](https://github.com/OWASP/DevSecOpsGuideline/blob/cd44d9778b68fe0122b7692df5942c948b2a87f9/LICENSE.md).
