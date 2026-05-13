import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { PlugZap, RefreshCw, TerminalSquare } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ServerConfig, TerminalOutputEvent, TerminalStatusEvent } from "../types";

interface TerminalViewProps {
  server: ServerConfig | null;
}

export default function TerminalView({ server }: TerminalViewProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);
  const [status, setStatus] = useState<TerminalStatusEvent["status"]>("disconnected");
  const [message, setMessage] = useState("Disconnected");

  useEffect(() => {
    const host = hostRef.current;
    if (!host || !server) {
      return;
    }

    let disposed = false;
    const terminal = new Terminal({
      cursorBlink: true,
      fontFamily: '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: 12,
      lineHeight: 1.18,
      theme: {
        background: "#07080b",
        foreground: "#eef1f6",
        cursor: "#51d88a",
        selectionBackground: "#57b9ff55",
        black: "#111318",
        red: "#ff6b7a",
        green: "#51d88a",
        yellow: "#ffcc66",
        blue: "#57b9ff",
        magenta: "#d98cff",
        cyan: "#5de4c7",
        white: "#eef1f6",
        brightBlack: "#697183",
        brightRed: "#ff8b96",
        brightGreen: "#71e5a2",
        brightYellow: "#ffd987",
        brightBlue: "#82caff",
        brightMagenta: "#e2a8ff",
        brightCyan: "#8aeddc",
        brightWhite: "#ffffff",
      },
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(host);
    fitAddon.fit();

    terminalRef.current = terminal;
    fitRef.current = fitAddon;
    const sessionId = crypto.randomUUID();
    sessionRef.current = sessionId;
    setStatus("connecting");
    setMessage("Opening SSH PTY");
    terminal.writeln(`Connecting to ${server.name}...`);

    const connect = async () => {
      const connectedSessionId = await invoke<string>("terminal_connect", {
        serverId: server.id,
        sessionId,
        cols: terminal.cols,
        rows: terminal.rows,
      });
      if (disposed) {
        await invoke("terminal_disconnect", { sessionId: connectedSessionId });
        return;
      }
    };

    const dataDisposable = terminal.onData((data) => {
      const sessionId = sessionRef.current;
      if (!sessionId) return;
      void invoke("terminal_input", { sessionId, data });
    });

    const resize = () => {
      const sessionId = sessionRef.current;
      fitAddon.fit();
      if (!sessionId) return;
      void invoke("terminal_resize", {
        sessionId,
        cols: terminal.cols,
        rows: terminal.rows,
      });
    };
    const observer = new ResizeObserver(resize);
    observer.observe(host);

    const unlistenersPromise = Promise.all([
      listen<TerminalOutputEvent>("terminal-output", (event) => {
        if (event.payload.sessionId === sessionRef.current) {
          terminal.write(event.payload.data);
        }
      }),
      listen<TerminalStatusEvent>("terminal-status", (event) => {
        if (event.payload.sessionId !== sessionRef.current) return;
        setStatus(event.payload.status);
        setMessage(event.payload.message);
      }),
    ]);

    void unlistenersPromise.then(() => connect()).catch((error) => {
      setStatus("reconnecting");
      setMessage(error instanceof Error ? error.message : String(error));
    });

    return () => {
      disposed = true;
      observer.disconnect();
      dataDisposable.dispose();
      void unlistenersPromise.then((unlisteners) => unlisteners.forEach((unlisten) => unlisten()));
      const sessionId = sessionRef.current;
      sessionRef.current = null;
      if (sessionId) {
        void invoke("terminal_disconnect", { sessionId });
      }
      terminal.dispose();
      terminalRef.current = null;
      fitRef.current = null;
    };
  }, [server]);

  if (!server) {
    return (
      <main className="content">
        <div className="empty-state">
          <TerminalSquare size={28} />
          <h2>No server selected</h2>
        </div>
      </main>
    );
  }

  return (
    <main className="content terminal-content">
      <header className="dashboard-header">
        <div>
          <p className="eyebrow">SSH PTY</p>
          <h2>Terminal</h2>
          <span className="server-target">{server.sshUser}@{server.host}:{server.sshPort}</span>
        </div>
        <span className={`health-pill ${status}`}>
          {status === "reconnecting" ? <RefreshCw size={14} className="spin" /> : <PlugZap size={14} />}
          {message}
        </span>
      </header>
      <section className="terminal-panel">
        <div ref={hostRef} className="terminal-host" />
      </section>
    </main>
  );
}
