# Orbom

[한국어](#한국어) · [English](#english)

![Orbom의 오브가 16초 동안 색을 한 바퀴 바꾸는 모습 / The Orbom orb cycling through its colors over 16 seconds](assets/orb-colors.png)

---

## 한국어

Rust로 만든 Windows 화면 이미지 검색 도구입니다. 화면의 궁금한 곳을 동그라미로 그리면 Google 이미지 검색 결과를 보여 줍니다. 별도 메인 화면 없이 트레이에 상주하며, 한국어와 영어 UI를 지원합니다.

화면 위 실행 버튼과 로딩 화면은 같은 유리 구슬 모양의 오브입니다. 원 전체의 색이 약 16초에 한 바퀴씩 천천히 바뀌고, 동그라미를 그리는 선도 그 순간의 오브와 같은 색으로 그려집니다. 오브는 매 프레임 실시간으로 계산하며, 바탕화면 버튼은 CPU를 한 코어의 1% 이하로 씁니다.

### 설치와 제거

[Releases](https://github.com/hscho0048/orbom-circle-to-search/releases)에서 설치 파일을 받아 실행합니다(직접 빌드했다면 `dist` 폴더). 관리자 권한은 필요 없고, 별도 런타임 설치도 필요 없습니다.

- `Orbom-Setup-x64.exe`: 일반 Intel/AMD PC
- `Orbom-Setup-arm64.exe`: ARM64 Windows PC (Snapdragon 등)

설치 위치는 `%LOCALAPPDATA%\Programs\Orbom`이며, 시작 메뉴 바로가기와 Windows 시작 시 자동 실행을 등록합니다. 자동 실행은 트레이 메뉴에서 끌 수 있습니다. 이미 설치되어 있으면 새 버전으로 업데이트하며, 실행 중인 Orbom은 자동으로 종료했다가 다시 시작합니다. `/S`로 실행하면 묻지 않고 설치합니다.

제거는 **Windows 설정 > 앱 > 설치된 앱 > Orbom**에서 합니다. 프로그램, 바로가기, 자동 실행, 설정, 검색 창 로그인 정보(`%LOCALAPPDATA%\Orbom`)를 모두 지웁니다.

실행하려면 Microsoft Edge WebView2 Runtime이 필요합니다. Windows 10/11에는 대부분 기본으로 설치되어 있습니다.

### 사용

1. **Ctrl + Shift + Space**를 누르거나, 화면의 오브 버튼 또는 시작 메뉴의 Orbom을 누릅니다.
2. 궁금한 대상을 동그라미로 둘러싸고 손을 뗍니다.
3. 짧은 리플 효과 뒤 모바일 Google의 **시각적으로 일치하는 항목**이 열립니다.

마우스·터치·펜으로 그릴 수 있고, 오브 버튼은 드래그해서 옮길 수 있습니다.

선택 화면에서 `Esc`는 선택 취소/닫기, `R`은 사각형, `L`은 동그라미, `K`는 키보드 선택입니다. 키보드 선택은 방향키로 이동, `Shift + 방향키`로 크기 변경, `Ctrl`을 함께 누르면 10px 단위, `Enter`로 검색합니다.

### 서피스 펜 버튼에 할당하기

서피스 펜 윗부분의 바로 가기 버튼(지우개 쪽 버튼)으로 Orbom을 바로 열 수 있습니다.

1. **설정 > Bluetooth 및 장치 > 펜 및 Windows Ink**를 엽니다.
2. **펜 바로 가기**에서 한 번 클릭 / 두 번 클릭 / 길게 누르기 중 하나를 고릅니다.
3. 다음 중 하나로 설정합니다.
   - **키보드 바로 가기 보내기** → `Ctrl + Shift + Space` (Orbom에서 다른 단축키를 골랐다면 그 조합). 가장 빠릅니다.
   - **앱 열기** → **Orbom**. Orbom이 꺼져 있어도 켜지면서 바로 선택 화면이 열립니다.

**앱 열기** 목록에는 패키지(MSIX) 앱만 나옵니다. Orbom을 목록에 넣으려면 설치 후 저장소에서 `package\register.ps1`을 한 번 실행하세요. 자체 서명 인증서를 만들어 이 PC에서 신뢰하도록 등록하므로(관리자 권한 한 번) Windows SDK가 필요합니다. `-Unregister`로 되돌립니다.

펜 옆면 버튼은 Windows가 오른쪽 클릭·지우개 용도로 쓰기 때문에 앱을 할당할 수 없습니다.

### 트레이 메뉴

트레이 아이콘이나 오브 버튼을 오른쪽 클릭하면 다음을 바꿀 수 있습니다. 모든 설정은 저장됩니다.

| 항목 | 내용 |
|---|---|
| 바탕화면에 오브 버튼 표시 | 오브 버튼 숨기기/보이기 |
| 오브 색 변화 | 색 애니메이션 켜기/끄기 (Windows 애니메이션 설정과 별개) |
| 오브 투명도 | 불투명 / 보통 / 투명 |
| 선택할 때 화면 어둡게 | 없음 / 약하게 / 보통 |
| 단축키 | Ctrl + Shift + Space, Ctrl + Alt + S, Alt + Shift + S |
| 언어 / Language | 한국어 / English (기본값은 Windows 표시 언어) |
| Windows 시작 시 실행 | 자동 실행 켜기/끄기 |

### 검색 동작과 개인정보

Google Lens의 일반 웹 이미지 업로드 화면에 선택 이미지를 전달합니다. 업로드에 성공한 뒤에만 모바일 User-Agent로 전환하고, 실제 검색 결과의 ‘시각적으로 일치하는 항목’ 링크를 엽니다.

- 확인·복사·붙여넣기·임시 이미지 호스팅 단계가 없습니다.
- 원을 감싸는 **사각형 영역 전체**를 PNG로 만들어 Google에 보냅니다. 원 바깥의 모서리 부분도 포함됩니다.
- 캡처 이미지를 파일로 저장하지 않으며, 캡처 메모리는 사용 후 지웁니다. WebView2 프로필은 `%LOCALAPPDATA%\Orbom\WebView2`에 저장되며, Google 서비스와 브라우저의 데이터 보관 정책이 적용됩니다.
- 검색 창의 웹페이지가 요청하는 카메라·마이크·위치 등 권한은 묻지 않고 모두 거절합니다.
- 인증·동의·네트워크 오류는 Google 웹 화면에서 확인할 수 있고, 연결에 실패하면 다시 시도할 수 있습니다. Google의 웹 업로드 화면이 바뀌면 연동을 수정해야 할 수 있습니다.

### 빌드

Windows 10/11, Rust stable MSVC(`x86_64`, `aarch64` 타깃), Visual Studio C++ Build Tools(ARM64 빌드 도구 포함), Windows SDK가 필요합니다.

```powershell
cargo test
cargo clippy --workspace --all-targets -- -D warnings
./build.ps1            # x64와 ARM64 설치 파일을 모두 dist에 만듭니다
./build.ps1 x64        # 하나만 만들 때
```

`build.ps1`은 아키텍처마다 앱(`orbom`)을 먼저 빌드한 뒤, 그 실행 파일과 라이선스 파일을 내장한 설치 프로그램(`installer/`, `orbom-setup`)을 빌드해 `dist\Orbom-Setup-<arch>.exe`로 복사합니다. C 런타임을 정적으로 링크하므로(`.cargo/config.toml`) Visual C++ 재배포 패키지가 필요 없습니다. 앱과 설치 프로그램의 아이콘은 빌드할 때 `src/art.rs`의 오브로 생성합니다.

개발 빌드 전용 옵션: `--preview-overlay`, `--preview-loading`, `--preview-bubble`은 실행 중인 앱과 별개로 각 화면을 띄웁니다. `--smoke-search`는 실제 화면 대신 생성한 귤 그림으로 업로드부터 모바일 이미지 결과까지 검증하고, 기록을 `target\smoke-search.json`에 남깁니다.

### 한계

HDR·보호된 동영상·보안 데스크톱은 캡처가 제한될 수 있습니다. ARM64 실기기, 실제 터치/펜 장치, 배율이 서로 다른 다중 모니터는 별도로 실기기 검증이 필요합니다.

### 라이선스

Copyright (c) 2026 조호성

Orbom은 다음 두 라이선스 중 원하는 것을 선택해 사용할 수 있는 **이중 라이선스**입니다.

- MIT 라이선스 ([LICENSE-MIT](LICENSE-MIT))
- Apache 라이선스 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

별도로 명시하지 않는 한, 이 프로젝트에 기여한 내용은 추가 조건 없이 위와 같은 이중 라이선스로 제공되는 것으로 봅니다.

사용한 기술의 라이선스는 아래 [서드파티 라이선스](#서드파티-라이선스--third-party-licenses)에 정리했습니다.

---

## English

A Windows screen image search tool written in Rust. Circle anything on your screen and Orbom shows Google image results for it. It lives in the system tray with no main window, and its UI is available in Korean and English.

The on-screen launch button and the loading screen share the same glass-bead orb. The whole orb slowly shifts color, one full lap about every 16 seconds, and the stroke you draw uses the orb's current color. The orb is rendered in real time every frame; the desktop button uses under 1% of one CPU core.

### Install and uninstall

Download an installer from [Releases](https://github.com/hscho0048/orbom-circle-to-search/releases) and run it (or use the `dist` folder if you built it yourself). No administrator rights or extra runtimes are needed.

- `Orbom-Setup-x64.exe`: regular Intel/AMD PCs
- `Orbom-Setup-arm64.exe`: ARM64 Windows PCs (Snapdragon and others)

Orbom installs to `%LOCALAPPDATA%\Programs\Orbom`, adds a Start menu shortcut and starts with Windows. You can turn off starting with Windows from the tray menu. If Orbom is already installed, the installer updates it, closing and restarting the running copy. Run it with `/S` to install without prompts.

To uninstall, go to **Settings > Apps > Installed apps > Orbom**. This removes the program, shortcut, startup entry, settings and search sign-in data (`%LOCALAPPDATA%\Orbom`).

Orbom needs the Microsoft Edge WebView2 Runtime, which comes preinstalled on most Windows 10/11 PCs.

### Usage

1. Press **Ctrl + Shift + Space**, or click the on-screen orb button or Orbom in the Start menu.
2. Circle what you want to look up and let go.
3. After a short ripple, Google's mobile **Visual matches** page opens.

You can draw with a mouse, touch or pen. Drag the orb button to move it.

On the selection screen, `Esc` clears the selection or closes, `R` switches to a rectangle, `L` to a circle and `K` to keyboard selection. With keyboard selection, the arrow keys move the area, `Shift + arrows` resize it, holding `Ctrl` moves in 10px steps, and `Enter` searches.

### Assigning it to the Surface Pen button

You can open Orbom with the shortcut button on top of a Surface Pen (the eraser end).

1. Open **Settings > Bluetooth & devices > Pen & Windows Ink**.
2. Under **Pen shortcuts**, pick single-click, double-click or press and hold.
3. Set it to one of these:
   - **Send keyboard shortcut** → `Ctrl + Shift + Space` (or whichever shortcut you chose in Orbom). This is the fastest.
   - **Open an app** → **Orbom**. Even if Orbom isn't running, it starts and opens the selection screen right away.

The **Open an app** list only shows packaged (MSIX) apps. To add Orbom to it, run `package\register.ps1` from the repository once after installing. It creates a self-signed certificate and trusts it on this PC (one administrator prompt), so it needs the Windows SDK. `-Unregister` undoes it.

The pen's side (barrel) button can't be assigned to apps, because Windows uses it for right-click and erasing.

### Tray menu

Right-click the tray icon or the orb button to change the following. Every setting is saved.

| Item | What it does |
|---|---|
| Show orb button on desktop | Show or hide the orb button |
| Animate orb colors | Turn the color animation on or off (independent of Windows' animation setting) |
| Orb transparency | Solid / Medium / Clear |
| Dim screen while selecting | Off / Light / Medium |
| Shortcut | Ctrl + Shift + Space, Ctrl + Alt + S, Alt + Shift + S |
| 언어 / Language | 한국어 / English (defaults to the Windows display language) |
| Start with Windows | Turn starting with Windows on or off |

### How search works, and privacy

Orbom hands the selected image to Google Lens's regular web upload page. Only after the upload succeeds does it switch to a mobile user agent and open the real results page's "Visual matches" link.

- There are no confirm, copy, paste or temporary image hosting steps.
- The **whole rectangle** around your circle is sent to Google as a PNG, including the corners outside the circle.
- Captures are never saved to disk, and capture memory is wiped after use. The WebView2 profile lives in `%LOCALAPPDATA%\Orbom\WebView2` and is subject to Google's and the browser's data retention policies.
- Permission requests from the page in the search window (camera, microphone, location and so on) are always denied without asking.
- Sign-in, consent and network errors show up in the Google page, and you can retry if the connection fails. If Google changes its web upload page, the integration may need updating.

### Building

You need Windows 10/11, stable Rust for MSVC (`x86_64` and `aarch64` targets), Visual Studio C++ Build Tools (including the ARM64 build tools) and the Windows SDK.

```powershell
cargo test
cargo clippy --workspace --all-targets -- -D warnings
./build.ps1            # builds both the x64 and ARM64 installers into dist
./build.ps1 x64        # just one
```

For each architecture, `build.ps1` builds the app (`orbom`) first. It then builds the installer (`installer/`, `orbom-setup`), which embeds that executable and the license files, and copies it to `dist\Orbom-Setup-<arch>.exe`. The C runtime is linked statically (`.cargo/config.toml`), so the Visual C++ Redistributable isn't needed. The app and installer icons are generated at build time from the orb in `src/art.rs`.

Debug-build-only options: `--preview-overlay`, `--preview-loading` and `--preview-bubble` open each screen separately from a running copy. `--smoke-search` checks the whole flow from upload to mobile image results using a generated tangerine picture instead of your screen, and writes a log to `target\smoke-search.json`.

### Limitations

HDR content, protected video and the secure desktop may not be capturable. ARM64 hardware, real touch/pen devices and multiple monitors with different scaling still need testing on real hardware.

### License

Copyright (c) 2026 조호성

Orbom is **dual-licensed**; you may use it under either of:

- the MIT License ([LICENSE-MIT](LICENSE-MIT))
- the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

Unless you explicitly state otherwise, any contribution you intentionally submit to this project is dual-licensed as above, without any additional terms or conditions.

Licenses of the technologies Orbom uses are listed under [Third-party licenses](#서드파티-라이선스--third-party-licenses) below.

---

## 서드파티 라이선스 / Third-party licenses

Orbom 실행 파일에는 아래 Rust 크레이트가 포함됩니다. 각 라이선스 전문은 [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt)에 있으며, 설치할 때 프로그램 폴더에도 함께 복사됩니다.

The Orbom executables include the Rust crates below. Their full license texts are in [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt), which the installer also copies into the program folder.

| 크레이트 / Crate | 버전 / Version | 라이선스 / License | 용도 / Used for |
|---|---|---|---|
| wry | 0.55.1 | Apache-2.0 OR MIT | WebView2 검색 창 / search window |
| webview2-com, webview2-com-sys, webview2-com-macros | 0.38.2, 0.38.2, 0.8.1 | MIT | WebView2 바인딩 / bindings |
| windows, windows-core, windows-collections, windows-future, windows-numerics, windows-result, windows-strings, windows-threading, windows-version, windows-link, windows-implement, windows-interface | 0.61.x 외 / and related | MIT OR Apache-2.0 | Windows API, COM |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | Win32 API |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | 창 핸들 / window handles |
| dpi | 0.1.2 | Apache-2.0 AND MIT | 화면 배율 / display scaling |
| png | 0.18.1 | MIT OR Apache-2.0 | 캡처 PNG 인코딩 / capture encoding |
| base64 | 0.22.1 | MIT OR Apache-2.0 | 이미지 전달 / image transfer |
| flate2, miniz_oxide, fdeflate, crc32fast, adler2, simd-adler32 | 1.1.10, 0.8.9/0.9.1, 0.3.7, 1.5.2, 2.0.1, 0.3.10 | MIT OR Apache-2.0 (miniz_oxide: + Zlib, adler2: + 0BSD, simd-adler32: MIT) | 압축 / compression |
| http, bytes, cookie, itoa | 1.5.0, 1.12.1, 0.18.2, 1.0.18 | MIT OR Apache-2.0 (bytes: MIT) | wry HTTP 타입 / HTTP types |
| time, time-core, time-macros, deranged, num-conv, powerfmt | 0.3.55, 0.1.9, 0.2.32, 0.5.8, 0.2.2, 0.2.0 | MIT OR Apache-2.0 | 시간 처리 / time handling |
| thiserror, thiserror-impl, once_cell, cfg-if, bitflags, dunce | 2.0.20, 2.0.20, 1.21.4, 1.0.5, 2.13.2, 1.0.5 | MIT OR Apache-2.0 (dunce: CC0-1.0 OR MIT-0 OR Apache-2.0) | 공용 유틸리티 / utilities |
| proc-macro2, quote, syn, unicode-ident | 1.0.107, 1.0.47, 2.0.119/3.0.6, 1.0.26 | MIT OR Apache-2.0 (unicode-ident: + Unicode-3.0) | 빌드 시 매크로 / build-time macros |

그 밖에 사용하는 기술 / Other technologies:

| 기술 / Technology | 라이선스·약관 / License or terms | 비고 / Notes |
|---|---|---|
| Rust 표준 라이브러리 / Rust standard library | MIT OR Apache-2.0 | 실행 파일에 포함 / linked into the executables |
| Microsoft C/C++ 런타임 (MSVC, UCRT) / C/C++ runtime | Microsoft Visual Studio 라이선스 (재배포 허용) / Visual Studio license (redistributable) | 정적 링크 / statically linked |
| Microsoft Edge WebView2 Runtime | Microsoft 소프트웨어 사용 조건 / Microsoft Software License Terms | 포함하지 않음, Windows에 설치된 것을 사용 / not bundled; uses the copy installed with Windows |
| Google Lens / Google 검색 / Google Search | Google 서비스 약관 / Google Terms of Service | 웹 서비스 / web service |
| 맑은 고딕, Segoe UI / Malgun Gothic, Segoe UI | Microsoft 글꼴 라이선스 / Microsoft font license | Windows 기본 글꼴, 포함하지 않음 / Windows system fonts, not bundled |

Google, Google Lens는 Google LLC의 상표이고, Windows, Surface, WebView2, Segoe, 맑은 고딕은 Microsoft Corporation의 상표입니다. Orbom은 Google 또는 Microsoft와 관련이 없습니다.

Google and Google Lens are trademarks of Google LLC. Windows, Surface, WebView2, Segoe and Malgun Gothic are trademarks of Microsoft Corporation. Orbom is not affiliated with Google or Microsoft.
