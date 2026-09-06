import { useCallback, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { AttachmentInfo } from '../../../api/types';
import { Dialog } from '../../Dialog';
import { WaveformPlayer } from '../WaveformPlayer';

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function AttachmentPreview({ attachment }: { attachment: AttachmentInfo }) {
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const isImage = attachment.content_type.startsWith('image/');
  const isVideo = attachment.content_type.startsWith('video/');
  const isAudio = attachment.content_type.startsWith('audio/');

  if (isImage) {
    return (
      <>
        <button onClick={() => setLightboxOpen(true)} className="block cursor-zoom-in">
          <img
            src={attachment.url}
            alt={attachment.filename}
            className="max-h-[300px] max-w-[400px] rounded border border-border object-contain"
            loading="lazy"
          />
        </button>
        {lightboxOpen && (
          <ImageLightbox
            url={attachment.url}
            filename={attachment.filename}
            onClose={() => setLightboxOpen(false)}
          />
        )}
      </>
    );
  }

  if (isVideo) {
    return (
      <div className="max-w-[480px]">
        <video
          src={attachment.url}
          controls
          preload="metadata"
          className="max-h-[360px] w-full rounded border border-border"
        />
        <div className="mt-1 text-xs text-text-muted">{attachment.filename} — {formatFileSize(attachment.file_size)}</div>
      </div>
    );
  }

  if (isAudio) {
    return (
      <WaveformPlayer
        src={attachment.url}
        filename={attachment.filename}
        fileSize={attachment.file_size}
      />
    );
  }

  return (
    <a
      href={attachment.url}
      target="_blank"
      rel="noopener noreferrer"
      className="flex items-center gap-2 rounded border border-border bg-bg-secondary px-3 py-2 text-sm transition-colors hover:bg-bg-hover"
    >
      <svg className="h-5 w-5 shrink-0 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" />
      </svg>
      <div className="min-w-0">
        <div className="truncate font-medium text-text-primary">{attachment.filename}</div>
        <div className="text-xs text-text-muted">{formatFileSize(attachment.file_size)}</div>
      </div>
      <svg className="h-4 w-4 shrink-0 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
      </svg>
    </a>
  );
}

export function ImageLightbox({ url, filename, onClose }: { url: string; filename: string; onClose: () => void }) {
  const [scale, setScale] = useState(1);
  const [translate, setTranslate] = useState({ x: 0, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const dragging = useRef(false);
  const lastPos = useRef({ x: 0, y: 0 });

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    setScale((s) => Math.min(Math.max(0.25, s - e.deltaY * 0.001), 10));
  }, []);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (e.button !== 0) return;
    dragging.current = true;
    setIsDragging(true);
    lastPos.current = { x: e.clientX, y: e.clientY };
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, []);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!dragging.current) return;
    const dx = e.clientX - lastPos.current.x;
    const dy = e.clientY - lastPos.current.y;
    lastPos.current = { x: e.clientX, y: e.clientY };
    setTranslate((t) => ({ x: t.x + dx, y: t.y + dy }));
  }, []);

  const handlePointerUp = useCallback(() => {
    dragging.current = false;
    setIsDragging(false);
  }, []);

  const resetView = useCallback(() => {
    setScale(1);
    setTranslate({ x: 0, y: 0 });
  }, []);

  return createPortal(
    <Dialog
      label={`Image viewer: ${filename}`}
      onClose={onClose}
      backdropClassName="bg-black/80"
      panelClassName="relative flex h-full w-full items-center justify-center"
    >
      {/* Top bar */}
      <div className="absolute top-0 left-0 right-0 flex items-center justify-between px-4 py-3 text-white">
        <span className="truncate text-sm font-medium">{filename}</span>
        <div className="flex items-center gap-2">
          <a
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded p-1.5 hover:bg-white/10"
            title="Open original"
          >
            <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
            </svg>
          </a>
          <button onClick={onClose} className="rounded p-1.5 hover:bg-white/10" title="Close">
            <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>
      {/* Zoom controls */}
      <div className="absolute bottom-4 left-1/2 flex -translate-x-1/2 items-center gap-1 rounded-lg bg-black/60 px-2 py-1 text-white">
        <button onClick={() => setScale((s) => Math.max(0.25, s / 1.5))} className="px-2 py-1 hover:bg-white/10 rounded" title="Zoom out">−</button>
        <button onClick={resetView} className="px-2 py-1 text-xs hover:bg-white/10 rounded" title="Reset zoom">{Math.round(scale * 100)}%</button>
        <button onClick={() => setScale((s) => Math.min(10, s * 1.5))} className="px-2 py-1 hover:bg-white/10 rounded" title="Zoom in">+</button>
      </div>
      {/* Image */}
      <img
        src={url}
        alt={filename}
        className="max-h-[90vh] max-w-[90vw] select-none"
        style={{
          transform: `translate(${translate.x}px, ${translate.y}px) scale(${scale})`,
          cursor: isDragging ? 'grabbing' : 'grab',
        }}
        draggable={false}
        onWheel={handleWheel}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
      />
    </Dialog>,
    document.body,
  );
}
