import { useState } from 'react';
import { safeExternalHttpsUrl, safeSameOriginUploadUrl } from '../utils/externalUrl';

interface ExternalImageProps {
  src: string;
  alt: string;
  label: string;
  className: string;
  privacyScopeKey: string;
}

/** Defers third-party image traffic until the reader explicitly accepts it. */
export function ExternalImage({ src, alt, label, className, privacyScopeKey }: ExternalImageProps) {
  const localSrc = safeSameOriginUploadUrl(src);
  const safeSrc = localSrc ?? safeExternalHttpsUrl(src);
  const stateKey = `${privacyScopeKey}\n${safeSrc ?? ''}`;
  const [allowedKey, setAllowedKey] = useState<string | null>(null);
  const [failedKey, setFailedKey] = useState<string | null>(null);
  const allowed = allowedKey === stateKey;
  const failed = failedKey === stateKey;

  if (!safeSrc) return null;
  if (localSrc) {
    if (failed) return null;
    return <img src={localSrc} alt={alt} className={className} referrerPolicy="no-referrer" onError={() => setFailedKey(stateKey)} />;
  }
  if (!allowed) {
    return (
      <div className="rounded border border-border bg-bg-primary/40 p-2 text-xs text-text-muted">
        <p>External image from {new URL(safeSrc).hostname}. Loading it shares your IP address with that site.</p>
        <button type="button" onKeyDown={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); setAllowedKey(stateKey); }} className="mt-1 rounded bg-bg-hover px-2 py-1 text-text-primary hover:bg-accent/20">
          Load external image: {label}
        </button>
      </div>
    );
  }
  if (failed) {
    return <a href={safeSrc} target="_blank" rel="noopener noreferrer" className="text-xs text-blue-400 underline">Open external image: {label}</a>;
  }
  return <img src={safeSrc} alt={alt} className={className} referrerPolicy="no-referrer" onError={() => setFailedKey(stateKey)} />;
}
