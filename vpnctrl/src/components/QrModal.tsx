import { X } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";

interface QrModalProps {
  title: string;
  link: string | null;
  onClose: () => void;
}

export default function QrModal({ title, link, onClose }: QrModalProps) {
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
        <code>{link}</code>
      </div>
    </div>
  );
}
