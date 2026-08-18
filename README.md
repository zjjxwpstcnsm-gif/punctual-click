# Punctual

> A local-first scheduled click assistant built with Rust and GPUI.
>
> 本地优先的准点点击助手：在指定的年月日、时分秒和毫秒，验证并点击网页中的目标按钮。

Punctual 面向需要在明确时间完成一次网页操作的普通用户。它可以自动识别“购买”“提交”“结算”“确认”等可点击控件；当存在多个候选时，会展示上下文并要求用户确认。用户也可以手动输入按钮文案，但任务保存前必须验证该文案确实对应页面上的可见、已启用、可点击元素。

## 核心能力

- 多任务管理，支持精确到毫秒的本地时间和 IANA 时区。
- 自动推理目标按钮，并显示候选文案、上下文、可点击状态和置信度。
- 手动按钮文案验证，排除普通文本、禁用按钮和被遮挡元素。
- 目标指纹与执行前重新定位，不依赖单一脆弱 CSS Selector。
- 严格准点、准点开始等待两种执行模式。
- 点击前预热页面、滚动并布防，目标时刻仅派发单次点击。
- 成功、失败、已点击但未确认三类结果，记录实际派发时间和偏差。
- 支持点击后原页跳转、SPA 状态变化、新标签页和新窗口结果确认。
- SQLite 本地持久化，应用重启后恢复待执行任务。
- 长页面和候选列表独立滚动；URL、结果和执行日志可一键复制。

## 浏览器支持与选择顺序

Punctual 会自动检测已安装浏览器，并按照以下顺序选择：

1. 高级用户显式指定的浏览器；
2. Google Chrome；
3. 受支持的系统默认浏览器；
4. 其他已安装且受支持的浏览器；
5. 安装包自带的 Punctual 托管浏览器。

支持的浏览器包括 Chrome、Microsoft Edge、Brave、Arc（macOS）、Vivaldi、Chromium、Opera、Firefox，以及 macOS Safari。

不同浏览器使用隔离的 Punctual 用户数据目录，不会直接读取用户日常浏览器的默认 Profile。第一次访问需要登录的网站时，请在 Punctual 打开的浏览器窗口中完成登录，后续任务会复用该会话。

Safari 自动化由系统的 `safaridriver` 提供。Safari 未启用“允许远程自动化”时，Punctual 会显示原因并自动尝试下一款浏览器。

## 安装

### macOS

发布页提供 Apple Silicon ARM64 DMG：

1. 打开 `Punctual-0.1.0-alpha.5-macos-arm64.dmg`；
2. 将 `Punctual.app` 拖入 `Applications`；
3. 从“应用程序”启动。

当前 Alpha 构建采用 ad-hoc 签名，尚未使用 Apple Developer ID 公证。首次启动可能需要前往“系统设置 → 隐私与安全性”选择“仍要打开”。

### Windows

发布页提供：

- `Punctual-0.1.0-alpha.5-windows-x64-setup.exe`：当前用户安装程序；
- `Punctual-0.1.0-alpha.5-windows-x64-portable.zip`：解压后直接运行的便携版。

当前 Alpha 构建尚未使用 Authenticode 证书，Windows SmartScreen 可能显示未知发布者提示。请先核对发布页提供的 SHA-256。

## 创建任务

1. 新建任务并填写名称、HTTP/HTTPS URL、执行时间和时区。
2. 打开浏览器并完成必要登录。
3. 使用“自动检测”扫描目标按钮，或输入手工按钮文案。
4. 当出现多个候选时，使用“高亮”确认页面位置并选择目标。
5. 配置结果判定条件：URL 变化、成功文案或成功选择器。
6. 保存任务，等待状态依次进入 `Pending → Preparing → Armed → Executing`。
7. 执行结束后查看最终 URL、确认依据、实际点击时间和偏差。

## 精度说明

Punctual 支持毫秒字段，并在执行前完成页面加载、目标定位和滚动；临近目标时间后使用单调时钟等待。但桌面操作系统、浏览器主线程、网络和目标网站服务端都不是硬实时环境，因此软件不承诺绝对的 1 毫秒误差。

每次执行都会记录计划时间、命令派发时间和可观测偏差，方便用户判断具体环境下的实际效果。

## 安全边界

Punctual 的定位是：帮助用户在自己授权并可见的浏览器会话中，准时完成一次常规点击。

项目不提供以下能力：

- 绕过验证码、排队、限购或网站风控；
- 反检测、浏览器指纹伪装或隐身自动化；
- 批量账号、批量下单或重复点击攻击；
- 自动填写或保存密码、银行卡、支付凭据；
- 绕过网站条款、访问控制或法律限制。

用户应遵守目标网站服务条款和所在地法律。遇到验证码、支付确认或人工验证时，应由用户本人完成。

## 从源码构建

### 依赖

- Rust stable；
- macOS 12+ 或 Windows 10/11；
- 对应平台的原生开发工具；
- 可选：系统已安装的受支持浏览器。

```bash
cargo fmt --all -- --check
cargo test -p punctual-core
cargo test -p punctual-browser --features cdp
cargo test -p punctual-engine
cargo build --release -p punctual-app
```

开发运行：

```bash
cargo run -p punctual-app
```

高级覆盖项：

```text
PUNCTUAL_BROWSER=chrome|edge|brave|firefox|safari|managed
PUNCTUAL_CHROMIUM=/absolute/path/to/chromium
PUNCTUAL_MANAGED_BROWSER=/absolute/path/to/managed/browser
PUNCTUAL_GECKODRIVER=/absolute/path/to/geckodriver
PUNCTUAL_RESOURCES_DIR=/absolute/path/to/resources
```

## 工程结构

```text
crates/
├── punctual-app/       GPUI 桌面界面
├── punctual-core/      领域模型、状态机、时间与执行计划
├── punctual-storage/   SQLite Repository 与 migrations
├── punctual-browser/   浏览器发现、CDP/WebDriver、目标识别与结果确认
└── punctual-engine/    后台运行时、调度器、任务 Worker 与消息总线

scripts/
├── detect_buttons.js
├── highlight_button.js
└── probe_target.js
```

## 隐私

- 任务、结果和执行日志默认保存在本机 SQLite 数据库。
- 浏览器会话保存在 Punctual 独立用户数据目录。
- 应用不要求云账号，不会保存目标网站账号密码。
- 按钮识别在本地页面和本地进程内完成。

## 状态与兼容性

当前版本是 `0.1.0-alpha.5`。这是可安装、可执行的 Alpha 版本，尚未承诺稳定 API 或完整网站兼容性。iframe、强封装 Shadow DOM、浏览器扩展页面和站点自定义安全控件可能需要后续适配。

变更记录见 [CHANGELOG.md](CHANGELOG.md)。安全问题提交方式见 [SECURITY.md](SECURITY.md)。

## License

Apache License 2.0。第三方运行时和依赖保留各自许可证，详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
