# TranslateB — 원서 1차 번역 도우미

PDF·DOCX 원서를 넣으면 [OpenCode](https://github.com/anomalyco/opencode)의 무료 AI 모델로 조각조각 1차 번역을 하고,
사람(또는 다른 AI)이 최종 검토를 바로 시작할 수 있는 상태로 결과 폴더를 정리해 주는 윈도우 프로그램입니다.

- 설치 없이 폴더째 들고 다니는 휴대용 프로그램 (USB, 구글 드라이브·원드라이브 폴더에서 실행 가능)
- OpenCode CLI 를 프로그램 안에 넣어 쓰고, 켤 때 GitHub 에서 새 버전을 확인
- 무료 모델 목록은 실행할 때마다 OpenCode 에서 새로 받아옴
- 긴 원서를 약 5000자 조각으로 나눠 조각마다 새 세션으로 번역 → "계속" 입력이 필요 없음
- 요약·누락·코드 변경을 자동 검사하고, 걸리면 다시 시킴
- 언제 꺼도 이어서 진행, 다른 PC에서 이어서 해도 됨

## 처음 쓰는 방법

1. `TranslateB.exe` 를 실행합니다.
2. **[점검]** 화면에서
   - OpenCode 엔진, PDF 엔진이 없으면 **설치** 를 누릅니다. (배포 zip 에는 이미 들어 있습니다)
   - 모델이 초록불인지 보고, **시험하기** 로 시험 번역이 되는지 확인합니다.
3. **[새 번역]** 화면에 PDF 나 DOCX 를 끌어다 놓습니다.
   - 단원 목록이 나오면 번역할 단원만 고릅니다. (목차·색인은 기본으로 빠져 있습니다)
   - 규칙(프로그래밍 / 일반)과 모델을 고르고 **번역 시작** 을 누릅니다.
4. **[작업 목록]** 에서 진행 상황을 봅니다. 일시정지·중지·이어하기를 할 수 있습니다.
5. 끝나면 "검토 준비 완료" 가 뜹니다. **폴더 열기** 로 결과를 확인합니다.

## 결과 폴더

```
작업/<책 이름>/
├─ 번역/01.md, 02.md …      단원별 번역본
├─ 자동검사_보고서.md        조각별 길이 비율, 코드 보존, 미번역 의심 줄, 용어집과 다른 용어, 사용 모델
├─ 목차.md                  원문 단원·절 목록 (검토 때 누락 확인용)
├─ 실패목록.txt              3번 시도해도 실패한 조각
├─ 원문_조각/, 번역_조각/     조각별 원문과 번역
└─ 상태.json                진행 상황, 실행 중인 PC
```

검토를 맡길 때는 이 폴더를 통째로 넘기면 됩니다.

## 여러 PC에서 쓰기

- 프로그램 폴더를 클라우드 폴더나 USB 에 두면 어느 PC에서든 이어서 할 수 있습니다.
- 한 작업을 두 PC에서 동시에 돌리면 경고가 뜹니다. (같은 조각을 겹쳐 번역하지 않도록)
- OpenCode 의 로그인 정보와 기록은 `engine/home` 안에만 저장되고 PC 에 남지 않습니다.
  공용 PC에서 로그인했다면 끝날 때 **로그아웃 (작업 끝내기)** 를 누르세요. 무료 모델은 보통 로그인이 필요 없습니다.

## 번역 규칙 바꾸기

`rules/` 폴더의 `.md` 파일이 AI 에게 주는 규칙입니다. 복사해서 새 규칙을 만들면 [새 번역] 화면에 나타납니다.

- 파일 첫 줄에 `<!-- code-check -->` 가 있으면 원문 코드 줄이 그대로 남았는지 검사합니다.
- `english=한국어, english2=한국어2` 형식의 줄은 용어집으로 쓰이고, 보고서에서 용어 불일치를 찾을 때도 씁니다.

## 지원하지 않는 것

- 스캔(사진)으로 된 PDF (글자를 뽑을 수 없음)
- 원본 서식을 살린 DOCX 출력 (결과는 마크다운)
- 그림 속 글자 번역 (`[그림: 설명]` 으로만 표시)

## 직접 빌드하기

```
cargo build --release
powershell -ExecutionPolicy Bypass -File scripts\package.ps1
```

`dist\TranslateB.zip` 에 엔진까지 넣은 휴대용 묶음이 만들어집니다.
개발 중에는 `TRANSLATEB_HOME` 으로 프로그램 폴더를, `TRANSLATEB_MOCK=1` 로 가짜 번역 엔진을 쓸 수 있습니다.

## 함께 쓰는 오픈소스

- [OpenCode](https://github.com/anomalyco/opencode) — MIT License (`engine/LICENSE-opencode.txt`)
- [PDFium](https://pdfium.googlesource.com/pdfium/) ([pdfium-binaries](https://github.com/bblanchon/pdfium-binaries)) — BSD-3-Clause 등 (`engine/LICENSE-pdfium.txt`)
- 화면: [egui/eframe](https://github.com/emilk/egui)
