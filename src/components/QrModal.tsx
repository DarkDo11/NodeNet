import { Copy, X } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { useEffect, useState } from "react";

interface QrModalProps {
  title: string;
  link: string | null;
  onClose: () => void;
}

export default function QrModal({ title, link, onClose }: QrModalProps) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 2000);
    return () => window.clearTimeout(timer);
  }, [copied]);

  if (!link) return null;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="qr-modal" onClick={(event) => event.stopPropagation()}>
        <button className="icon-button modal-close" onClick={onClose} title="Close">
          <X size={17} />
        </button>
        <h3>{title}</h3>
        <div className="qr-box">
          <QRCodeSVG value={link} size={220} level="M" marginSize={2} />
        </div>
        <div className="qr-link-row">
          <code>{link}</code>
          <button
            className="icon-button"
            onClick={() => {
              void navigator.clipboard.writeText(link).then(() => setCopied(true));
            }}
            title="Copy link"
          >
            {copied ? <span className="copied-label">Copied!</span> : <Copy size={16} />}
          </button>
        </div>
      </div>
    </div>
  );
}
