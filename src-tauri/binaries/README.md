# Sidecar Binaries

이 디렉토리에는 Tauri 앱에 번들될 `plumise-agent` 실행 파일을 배치합니다.

## 파일명 규칙

Tauri는 플랫폼별로 다른 파일명을 찾습니다:

- **Windows**: `plumise-agent-x86_64-pc-windows-msvc.exe`
- **macOS (Intel)**: `plumise-agent-x86_64-apple-darwin`
- **macOS (Apple Silicon)**: `plumise-agent-aarch64-apple-darwin`
- **Linux**: `plumise-agent-x86_64-unknown-linux-gnu`

## 빌드 방법

plumise-agent 프로젝트에서:

```bash
# 현재 플랫폼용 빌드
cargo build --release

# 크로스 컴파일 (예: macOS에서 Windows용)
cargo build --release --target x86_64-pc-windows-msvc
```

빌드된 바이너리를 이 디렉토리에 복사하고 위 규칙에 맞게 이름을 변경합니다.

## Dev Mode 래퍼

개발 모드에서는 시스템 PATH의 `plumise-agent`를 호출하는 간단한 래퍼 스크립트를 사용합니다:

```bash
#!/bin/bash
exec plumise-agent "$@"
```

이 방식으로 개발 중에는 실제 바이너리를 복사하지 않고도 빌드가 가능합니다.

프로덕션 빌드 시에는 이 래퍼를 실제 바이너리로 교체해야 합니다.

## 이중 폴백 시스템

코드 레벨에서도 추가 폴백이 있습니다:
1. 먼저 sidecar 방식으로 실행 시도
2. sidecar 실행 실패 시 시스템 PATH에서 `plumise-agent` 실행

이중 폴백으로 개발 환경에서의 유연성을 보장합니다.

## 참고

- sidecar 바이너리는 Git에 커밋하지 않습니다 (크기 때문에)
- CI/CD에서 빌드 시 자동으로 생성하도록 설정 권장
- 각 플랫폼용 바이너리는 해당 플랫폼에서 빌드해야 함
