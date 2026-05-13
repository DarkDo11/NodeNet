import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { CheckCircle2, Clipboard, RefreshCw, XCircle } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { CommandOutputEvent } from "../types";

interface CommandOutputModalProps {
  title: string;
  serverId: string;
  command: string;
  onClose: () => void;
  onComplete?: (error: string | null) => void;
}

export default function CommandOutputModal({
  title,
  serverId,
  command,
  onClose,
  onComplete,
}: CommandOutputModalProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const outputRef = useRef("");
  const onCompleteRef = useRef(onComplete);
  const sessionIdRef = useRef(crypto.randomUUID());
  const completedRef = useRef(false);
  const [running, setRunning] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    onCompleteRef.current = onComplete;
  }, [onComplete]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    let disposed = false;
    sessionIdRef.current = crypto.randomUUID();
    completedRef.current = false;
    outputRef.current = "";
    setRunning(true);
    setError(null);
    const terminal = new Terminal({
      cursorBlink: false,
      disableStdin: true,
      fontFamily: '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: 12,
      lineHeight: 1.18,
      scrollback: 20_000,
      theme: {
        background: "#07080b",
        foreground: "#eef1f6",
        cursor: "#51d88a",
        selectionBackground: "#57b9ff55",
      },
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(host);
    fitAddon.fit();
    terminalRef.current = terminal;
    terminal.writeln(`$ ${command}`);
    terminal.writeln("");

    const observer = new ResizeObserver(() => fitAddon.fit());
    observer.observe(host);

    const finish = (nextError: string | null) => {
      if (disposed || completedRef.current) return;
      completedRef.current = true;
      setError(nextError);
      setRunning(false);
      onCompleteRef.current?.(nextError);
    };

    const unlistenPromise = listen<CommandOutputEvent>("command-output", (event) => {
      if (event.payload.sessionId !== sessionIdRef.current) return;
      if (event.payload.line) {
        terminal.writeln(event.payload.line);
        outputRef.current = `${outputRef.current}${event.payload.line}\n`;
      }
      if (event.payload.done) {
        finish(event.payload.line || null);
      }
    });

    void unlistenPromise
      .then(() =>
        disposed
          ? undefined
          : invoke("run_streaming_command", {
              serverId,
              command,
              sessionId: sessionIdRef.current,
            }),
      )
      .then(() => undefined)
      .catch((err) => {
        if (completedRef.current) return;
        const message = err instanceof Error ? err.message : String(err);
        terminal.writeln(message);
        outputRef.current = `${outputRef.current}${message}\n`;
        finish(message);
      });

    return () => {
      disposed = true;
      observer.disconnect();
      void unlistenPromise.then((unlisten) => unlisten());
      terminal.dispose();
      terminalRef.current = null;
    };
  }, [command, serverId]);

  const copyOutput = async () => {
    await navigator.clipboard.writeText(outputRef.current);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1400);
  };

  return (
    <div className="command-output-backdrop" role="dialog" aria-modal="true" aria-label={title}>
      <section className="command-output-modal">
        <header className="command-output-header">
          <div>
            <p className="eyebrow">SSH command</p>
            <h3>{title}</h3>
          </div>
          <div className="header-actions">
            <span className={error ? "health-pill offline" : running ? "health-pill connecting" : "health-pill connected"}>
              {error ? (
                <XCircle size={14} />
              ) : running ? (
                <RefreshCw size={14} className="spin" />
              ) : (
                <CheckCircle2 size={14} />
              )}
              {error ? "Error" : running ? "Running" : "Done"}
            </span>
            <button className="command-button" onClick={() => void copyOutput()}>
              <Clipboard size={16} />
              <span>{copied ? "Copied" : "Copy"}</span>
            </button>
            <button className="command-button" disabled={running} onClick={onClose}>
              Close
            </button>
          </div>
        </header>
        <div className="command-output-command">
          <code>{command}</code>
        </div>
        <div ref={hostRef} className="command-output-terminal" />
      </section>
    </div>
  );
}
