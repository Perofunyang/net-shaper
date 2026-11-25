# NetShaper

Windows 환경에서 실시간으로 네트워크 **지연(Lag)** 및 **대역폭(Bandwidth)** 제한 환경을 재현할 수 있는 경량 네트워크 테스트 도구입니다.  
[clumsy](https://github.com/jagt/clumsy)에서 **Lag + Bandwidth** 기능만 남기고 Rust와 Token Bucket 알고리즘으로 새롭게 구현했습니다.

---

## 다운로드 및 실행 방법 (Release)

1. [Releases](../../releases) 페이지에서 최신 버전 `.zip` 파일을 다운로드해 압축을 풉니다.
2. 폴더 안에 아래 **3개 파일이 반드시 함께** 있어야 합니다.
   * `net-shaper.exe`
   * `WinDivert.dll`
   * `WinDivert64.sys`
3. `net-shaper.exe`를 실행합니다. *(네트워크 드라이버 로드를 위해 관리자 권한 UAC 승인이 필요합니다.)*

> **Windows SmartScreen 경고창("Windows의 PC 보호")이 뜨는 경우**  
> 개인 빌드·서명되지 않은 실행 파일에 대해 표시되는 기본 경고입니다.  
> * 경고창에서 **`추가 정보` → `실행`**을 누르시거나,  
> * 압축 해제 전 `.zip` 파일 **`우클릭 → 속성 → 차단 해제` 체크** 후 압축을 풀어 실행하세요.

---

## 주요 기능

* **WinDivert 기반 실시간 패킷 필터링**
  * 별도 프록시 설정이나 대상 앱 코드 수정 없이 패킷을 가로채어 제어합니다.
  * `outbound`, `inbound`, `outbound or inbound` 빠른 프리셋과 WinDivert 전체 필터 문법(특정 IP, 포트, TCP/UDP 등)을 지원합니다.
  * `Start` 전 필터 문법 검증을 통해 잘못된 표현식을 즉시 안내합니다.
* **Lag (고정 지연)**
  * 송수신 스레드 분리 구조 + 고해상도 타이머(`spin_sleep`)로 지연 오차를 줄였습니다.
  * 지연 범위: `0 ~ 10,000 ms` (기본값 `150 ms`)
* **Bandwidth (Token Bucket 대역폭 셰이핑)**
  * 단순 패킷 폐기(Drop) 방식 대신 **가상 스케줄 기반 Token Bucket**으로 패킷 전송 시점을 조절해 속도를 매끄럽게 제한합니다. *(대기 시간 1.5초 초과, 버퍼 포화, 또는 `0 KB/s` 설정 시에만 드롭)*
  * 제한 범위: `0 ~ 100,000 KB/s` (기본값 `50 KB/s` / `0.40 Mbps`)
* **실시간 통계 (`STATISTICS`)**
  * `RX` / `TX` 패킷 수 및 실시간 전송률(`KB/s`), 현재 대기 중인 패킷(`Queued`), 버려진 패킷(`Dropped`)을 모니터링합니다.

---

## 필터링(Filtering) 사용 팁

> ⚠️ **중요:** `Functions`의 `Inbound`/`Outbound` 체크박스가 켜져 있어도, 상단 `Filter` 조건에서 해당 방향의 패킷을 잡지 않으면 적용되지 않습니다.
> * **업로드(나가는 트래픽)만 테스트:** `Filter` = `outbound` (기본값)
> * **다운로드 + 업로드 양방향 모두 테스트:** `Presets`에서 **`both`**(`outbound or inbound`) 선택
> * **특정 포트만 테스트 예시:**
>   ```text
>   outbound and (tcp.DstPort == 8080 or udp.DstPort == 8080)
>   ```

---

## 기술 스택 & 라이선스 참고

* **Language:** Rust
* **GUI:** `eframe` / `egui`
* **Timing:** `spin_sleep`
* **Packet Engine:** [WinDivert 2.2](https://reqrypt.org/windivert.html) (LGPL-3.0 / GPL-3.0)
* **License:** MIT
* **주의:** 온라인 게임 등 안티치트 드라이버가 실행 중인 환경에서는 WinDivert 드라이버 로드가 차단될 수 있으며, 테스트 목적 외의 비정상적인 네트워크 조작 용도로 사용하지 마세요.