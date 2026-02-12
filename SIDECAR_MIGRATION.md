# Tauri Sidecar 마이그레이션 완료

## 변경 사항 요약

plumise-agent-app의 Rust 백엔드를 `tokio::process::Command` 방식에서 Tauri의 공식 sidecar 시스템으로 마이그레이션했습니다.

## 주요 변경 파일

### `/Users/jskim/Desktop/vibe/plumise-agent-app/src-tauri/src/commands/agent.rs`

#### Import 변경
```rust
// 이전
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

// 이후
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
```

#### State 변경
```rust
pub struct AgentState {
    pub process: Option<CommandChild>,  // 이전: Option<Child>
    pub status: AgentStatus,
    pub http_port: u16,
}
```

#### start_agent 함수 주요 변경

1. **Sidecar 실행 (우선 시도)**
   ```rust
   let spawn_result = app
       .shell()
       .sidecar("plumise-agent")
       .and_then(|cmd| {
           let mut cmd = cmd;
           for (key, val) in &envs {
               cmd = cmd.env(key, val);
           }
           Ok(cmd)
       })
       .and_then(|cmd| cmd.spawn());
   ```

2. **이벤트 기반 로그 캡처**
   ```rust
   tokio::spawn(async move {
       while let Some(event) = rx.recv().await {
           match event {
               CommandEvent::Stdout(bytes) => {
                   if let Ok(line) = String::from_utf8(bytes) {
                       let _ = app_clone.emit("agent-log", LogEvent {
                           level: level.to_string(),
                           message: line,
                       });
                   }
               }
               CommandEvent::Stderr(bytes) => { /* ... */ }
               CommandEvent::Terminated(payload) => {
                   // 크래시 감지 및 상태 업데이트
                   guard.status = AgentStatus::Error;
                   guard.process = None;
                   break;
               }
               _ => {}
           }
       }
   });
   ```

3. **폴백 메커니즘 (dev 모드)**
   - Sidecar 실행 실패 시 시스템 PATH에서 `plumise-agent` 실행
   - 기존 tokio::process::Command 사용
   - 동일한 기능 유지 (stdout/stderr 캡처, 이벤트 emit)

#### stop_agent 함수 변경

```rust
// CommandChild.kill()은 self를 소비하므로 Option::take() 사용
if let Some(child) = guard.process.take() {
    let _ = child.kill();
}
```

#### poll_agent_health 함수 변경

- `child.try_wait()` 제거 (더 이상 필요 없음)
- `CommandEvent::Terminated`가 크래시를 자동으로 감지
- Health polling은 readiness 감지만 담당

### `/Users/jskim/Desktop/vibe/plumise-agent-app/src-tauri/tauri.conf.json`

```json
{
  "bundle": {
    "externalBin": ["binaries/plumise-agent"]
  }
}
```

### `/Users/jskim/Desktop/vibe/plumise-agent-app/src-tauri/binaries/`

새로 생성된 디렉토리:
- `README.md`: 바이너리 설정 가이드
- `.gitignore`: 바이너리는 제외, 래퍼만 포함
- `plumise-agent-x86_64-apple-darwin`: Dev 모드 래퍼 스크립트

```bash
#!/bin/bash
exec plumise-agent "$@"
```

## 기능 변경 사항

### 개선된 점

1. **이벤트 기반 프로세스 관리**
   - `CommandEvent::Terminated`로 크래시 자동 감지
   - `try_wait()` polling 제거로 성능 향상
   - 더 깔끔한 프로세스 라이프사이클 관리

2. **프로덕션 빌드 준비**
   - Tauri의 공식 sidecar 시스템 사용
   - 번들된 바이너리가 앱과 함께 배포됨
   - 크로스 플랫폼 빌드 지원

3. **이중 폴백 시스템**
   - Sidecar → 시스템 PATH 순으로 시도
   - 개발 환경에서 유연성 극대화

### 유지된 기능

- ✅ Preflight check (config validation, network test, port check)
- ✅ Health polling (readiness detection)
- ✅ Log events (stdout/stderr → frontend)
- ✅ Status events (Starting/Running/Stopping/Stopped/Error)
- ✅ Graceful shutdown (HTTP /shutdown → kill 폴백)
- ✅ Environment variable injection
- ✅ AgentConfig 구조 동일

## 빌드 및 배포

### Dev 모드
```bash
cd plumise-agent-app
npm run tauri dev
```

- 래퍼 스크립트가 시스템 PATH의 `plumise-agent` 호출
- 또는 sidecar 실패 시 코드 레벨 폴백 작동

### Production 빌드

1. **바이너리 준비**
   ```bash
   cd plumise-agent
   cargo build --release --target x86_64-apple-darwin
   cp target/x86_64-apple-darwin/release/plumise-agent \
      ../plumise-agent-app/src-tauri/binaries/plumise-agent-x86_64-apple-darwin
   ```

2. **앱 빌드**
   ```bash
   cd plumise-agent-app
   npm run tauri build
   ```

3. **크로스 플랫폼 (옵션)**
   - Windows: `plumise-agent-x86_64-pc-windows-msvc.exe`
   - Linux: `plumise-agent-x86_64-unknown-linux-gnu`
   - macOS ARM: `plumise-agent-aarch64-apple-darwin`

## 테스트 체크리스트

- [ ] Dev 모드에서 agent 시작/중지
- [ ] Logs 탭에서 stdout/stderr 출력 확인
- [ ] Health check가 "Running"으로 전환 확인
- [ ] Agent 크래시 시 Error 상태 전환 확인
- [ ] Graceful shutdown 동작 확인
- [ ] Force kill 폴백 동작 확인
- [ ] Preflight check 전체 통과 확인
- [ ] Production 빌드 후 번들된 바이너리 실행 확인

## 향후 작업

1. **CI/CD 자동화**
   - GitHub Actions에서 플랫폼별 바이너리 빌드
   - Tauri 앱 빌드 시 바이너리 자동 주입

2. **모니터링 개선**
   - Agent 메모리/CPU 사용량 표시
   - GPU 사용량 모니터링 (PyTorch)

3. **업데이트 시스템**
   - Agent 바이너리 독립 업데이트
   - 버전 불일치 감지 및 알림

## 참고 문서

- [Tauri Sidecar 공식 문서](https://v2.tauri.app/develop/sidecar/)
- [tauri-plugin-shell 문서](https://v2.tauri.app/plugin/shell/)
- [Cargo 크로스 컴파일](https://rust-lang.github.io/rustup/cross-compilation.html)
