# GitHub Issue: Antigravity CLI Bug Report

Paste the fields below directly into the issue form at:
👉 **https://github.com/google-antigravity/antigravity-cli/issues/new?template=bug_report.yml**

---

### Title
```text
[Bug]: WebChannel read loop leaks raw "Read loop (req %d): err=context canceled" to stderr on session refresh
```

---

### Antigravity CLI Version
```text
1.2.10
```

---

### Environment Context
```markdown
- **OS:** Microsoft Windows 11 Pro (Build 10.0.26200, 64-bit)
- **Shell:** PowerShell 7.6.6 (`pwsh.exe`)
- **Terminal Emulator:** Windows Terminal 1.24
- **Runtime Version:** Embedded Go runtime (`go1.27.1 windows/amd64`)
- **Hardware Details:** AMD Ryzen AI 9 HX 470 w/ Radeon 890M (x86_64)
```

---

### Steps to Reproduce
```markdown
1. Launch Antigravity CLI with remote control enabled:
   ```powershell
   agy --remote-control --dangerously-skip-permissions
   ```
2. Leave the session running actively or idle across the ~4-hour WebChannel backend token lifetime boundary (in our case, 3h 56m).
3. Observe raw unformatted error message emitted directly into the active interactive terminal window when the backend initiates connection rotation:
   ```text
   2026/09/24 11:05:01 Read loop (req 3581): err=context canceled
   ```
```

---

### Expected Behavior
```markdown
Routine WebChannel session expiration, channel rotation, transport upgrades (to WebRTC), and clean request cancellations (`context.Canceled`) should occur transparently in the background without polluting the interactive terminal console. 

Cancellations should be suppressed alongside `io.EOF`, and any legitimate transport diagnostics should be dispatched through the injected `WithLogger` (`webchannelLogger`) rather than standard library `log.Printf` writing to `os.Stderr`.
```

---

### Actual Behavior / Error Screenshots
```markdown
The raw Go standard library logger prints directly to `os.Stderr`, corrupting the interactive terminal display:

```text
2026/09/24 11:05:01 Read loop (req 3581): err=context canceled
```

This causes significant user confusion and trust issues, making users believe the ongoing conversation turn, context, or tool execution failed or crashed, even though the CLI recovers and reconnects automatically 360 ms later.
```

---

### Logs & Configurations
```markdown
#### Correlated CLI Log Timeline (`cli-20260924_070830.log`)
```log
W0924 11:05:01.409862   42646 remote_control_v2.go:2239] WebChannel: XMLHTTP Bad status 400 (65878)
E0924 11:05:01.409862   42646 remote_control_v2.go:1481] [remote-control-...-v2] WebChannel error callback triggered: 1
I0924 11:05:01.410370   42646 remote_control_v2.go:1395] [remote-control-...-v2] WebChannel OnClose callback triggered
I0924 11:05:01.410924     595 remote_control_v2.go:2071] [remote-control-...-v2] Connection loop exited: WebChannel connection closed
I0924 11:05:01.410924     595 remote_control_v2.go:2187] [remote-control-...-v2] Connection was healthy (lasted 3h56m24.1090402s). Resetting backoff.
I0924 11:05:01.410924     595 remote_control_v2.go:2190] [remote-control-...-v2] Connection ended. Retrying immediately...
I0924 11:05:01.410924     595 remote_control_v2.go:2194] [remote-control-...-v2] Connection ended with error: WebChannel connection closed
I0924 11:05:01.772629     595 remote_control_v2.go:2037] [remote-control-...-v2] Connection status: Connected
```

#### Binary Analysis & Root Cause
Disassembly of `agy.exe` (pclntab symbol `google3/net/webchannel/client/go/webchannel.goHttpRequestSend.func1`) demonstrates:
1. The read loop checks `if err == io.EOF` (via `runtime.ifaceeq`), but does NOT check for `errors.Is(err, context.Canceled)` during request aborts (`goHttpRequestAbort` / `Close()`).
2. The log call is hardcoded to Go's standard library `log.std` pointer (`mov rax, [rip + 0x94dfa38] ; call log.(*Logger).output`), bypassing the consumer's structured logger (`webchannelLogger` / `glog`) and writing straight to `os.Stderr`.

#### Suggested Fix
In `google3/net/webchannel/client/go/webchannel/webchannel.go`:
```diff
--- a/net/webchannel/client/go/webchannel/webchannel.go
+++ b/net/webchannel/client/go/webchannel/webchannel.go
@@ -xxx,xx +xxx,xx @@ func goHttpRequestSend(req *http.Request, ...) {
 		for {
 			n, err := resp.Body.Read(buf)
 			if err != nil {
-				if err != io.EOF {
-					log.Printf("Read loop (req %d): err=%v", reqId, err)
+				if err != io.EOF && !errors.Is(err, context.Canceled) {
+					if client.logger != nil {
+						client.logger.Warningf("Read loop (req %d): err=%v", reqId, err)
+					} else {
+						log.Printf("Read loop (req %d): err=%v", reqId, err)
+					}
 				}
 				return
 			}
```
```
