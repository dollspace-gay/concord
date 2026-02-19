import { useState, useRef, useEffect, useCallback } from 'react';

interface WaveformPlayerProps {
  src: string;
  filename: string;
  fileSize?: number;
}

/** Downsample audio buffer to a fixed number of amplitude bars. */
function getWaveformData(buffer: AudioBuffer, bars: number): number[] {
  const raw = buffer.getChannelData(0);
  const blockSize = Math.floor(raw.length / bars);
  const result: number[] = [];
  for (let i = 0; i < bars; i++) {
    let sum = 0;
    const start = i * blockSize;
    for (let j = 0; j < blockSize; j++) {
      sum += Math.abs(raw[start + j]);
    }
    result.push(sum / blockSize);
  }
  // Normalize to 0..1
  const max = Math.max(...result, 0.01);
  return result.map((v) => v / max);
}

const BAR_COUNT = 50;

export function WaveformPlayer({ src, filename, fileSize }: WaveformPlayerProps) {
  const [playing, setPlaying] = useState(false);
  const [progress, setProgress] = useState(0);
  const [duration, setDuration] = useState(0);
  const [waveform, setWaveform] = useState<number[]>([]);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const animRef = useRef<number>(0);

  // Decode audio to generate waveform
  useEffect(() => {
    let cancelled = false;
    const ctx = new AudioContext();
    fetch(src)
      .then((res) => res.arrayBuffer())
      .then((buf) => ctx.decodeAudioData(buf))
      .then((decoded) => {
        if (!cancelled) {
          setWaveform(getWaveformData(decoded, BAR_COUNT));
          setDuration(decoded.duration);
        }
      })
      .catch(() => {
        // Fallback: generate flat bars
        if (!cancelled) {
          setWaveform(Array(BAR_COUNT).fill(0.3));
        }
      })
      .finally(() => ctx.close());
    return () => { cancelled = true; };
  }, [src]);

  const updateProgressRef = useRef<() => void>(() => {});

  useEffect(() => {
    updateProgressRef.current = () => {
      const audio = audioRef.current;
      if (audio && !audio.paused) {
        setProgress(audio.currentTime / (audio.duration || 1));
        animRef.current = requestAnimationFrame(() => updateProgressRef.current());
      }
    };
  }, []);

  const togglePlay = useCallback(() => {
    const audio = audioRef.current;
    if (!audio) return;
    if (audio.paused) {
      audio.play();
      setPlaying(true);
      animRef.current = requestAnimationFrame(() => updateProgressRef.current());
    } else {
      audio.pause();
      setPlaying(false);
      cancelAnimationFrame(animRef.current);
    }
  }, []);

  const handleEnded = useCallback(() => {
    setPlaying(false);
    setProgress(0);
    cancelAnimationFrame(animRef.current);
  }, []);

  const handleBarClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const audio = audioRef.current;
    if (!audio || !audio.duration) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const frac = (e.clientX - rect.left) / rect.width;
    audio.currentTime = frac * audio.duration;
    setProgress(frac);
  }, []);

  useEffect(() => {
    return () => cancelAnimationFrame(animRef.current);
  }, []);

  const formatTime = (seconds: number) => {
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, '0')}`;
  };

  const currentTime = duration * progress;

  return (
    <div className="min-w-[280px] max-w-[400px] rounded border border-border bg-bg-secondary p-3">
      <audio ref={audioRef} src={src} preload="metadata" onEnded={handleEnded} />
      <div className="mb-2 truncate text-sm font-medium text-text-primary">{filename}</div>
      <div className="flex items-center gap-3">
        <button
          onClick={togglePlay}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-blue-600 text-white transition-colors hover:bg-blue-500"
        >
          {playing ? (
            <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24">
              <rect x="6" y="4" width="4" height="16" />
              <rect x="14" y="4" width="4" height="16" />
            </svg>
          ) : (
            <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24">
              <path d="M8 5v14l11-7z" />
            </svg>
          )}
        </button>
        {/* Waveform bars */}
        <div
          className="flex h-8 flex-1 cursor-pointer items-end gap-px"
          onClick={handleBarClick}
        >
          {waveform.map((amp, i) => {
            const barProgress = i / waveform.length;
            const isPlayed = barProgress <= progress;
            const height = Math.max(4, amp * 32);
            return (
              <div
                key={i}
                className={`flex-1 rounded-sm transition-colors ${
                  isPlayed ? 'bg-blue-400' : 'bg-text-muted/30'
                }`}
                style={{ height: `${height}px` }}
              />
            );
          })}
        </div>
      </div>
      <div className="mt-1 flex justify-between text-xs text-text-muted">
        <span>{formatTime(currentTime)}</span>
        <span>{formatTime(duration)}{fileSize ? ` — ${formatFileSize(fileSize)}` : ''}</span>
      </div>
    </div>
  );
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
