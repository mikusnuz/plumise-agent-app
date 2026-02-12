[English](README.md) | **한국어**

# Plumise Agent

Plumise 네트워크 추론 에이전트를 실행하기 위한 데스크톱 애플리케이션입니다. 원클릭 설정으로 GPU/CPU 컴퓨팅 파워를 Plumise 분산 추론 네트워크에 기여하고 PLM 보상을 획득합니다.

## 기능
- 원클릭 에이전트 관리 (시작/중지)
- 실시간 시스템 모니터링 (CPU, RAM, VRAM)
- 라이브 추론 메트릭 (요청, 토큰, 지연시간)
- 시작 전 자동 사전점검
- 검색, 필터, 내보내기 기능이 있는 실시간 로그 뷰어
- 내장 업데이터를 통한 자동 업데이트
- 모던 glassmorphism UI

## 다운로드
[GitHub Releases](https://github.com/mikusnuz/plumise-agent-app/releases)에서 최신 인스톨러를 다운로드합니다.
- Windows: `.msi` 또는 `.exe` (NSIS) 인스톨러

## 빠른 시작
1. 최신 릴리스를 다운로드하고 설치합니다.
2. Plumise Agent를 엽니다.
3. 설정 → 개인 키(0x...) 입력
4. 대시보드의 "시작" 클릭
5. 에이전트의 성능 및 보상을 모니터링합니다.

## 소스에서 빌드하기
### 필수 요구사항
- Node.js 20+
- Rust 1.77+
- Python 3.11+ (에이전트 사이드카 빌드용)

### 단계
```bash
git clone https://github.com/mikusnuz/plumise-agent-app.git
cd plumise-agent-app
npm install
npm run tauri:dev
npm run tauri:build
```

### 사이드카 빌드
```bash
git clone https://github.com/mikusnuz/plumise-agent.git
cd plumise-agent
pip install -r requirements.txt pyinstaller
pyinstaller plumise-agent.spec
```

## 아키텍처
- **Shell**: Tauri v2 (Rust + WebView)
- **Frontend**: React + TypeScript + Tailwind CSS v4 + Recharts
- **Agent**: PyInstaller 번들 plumise-agent (사이드카)
- **Chain**: Plumise Network (EVM 호환)

## 라이선스
MIT

## 링크
- [Plumise Chain](https://plumise.com)
- [Plumise Explorer](https://explorer.plumise.com)
- [Plumise Agent](https://github.com/mikusnuz/plumise-agent)
