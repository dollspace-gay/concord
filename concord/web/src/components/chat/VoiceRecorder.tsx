import { useState, useRef, useCallback, useEffect } from 'react';

interface VoiceRecorderProps {
  onRecorded: (blob: Blob) => void;
  onCancel: () => void;
}

export function VoiceRecorder({ onRecorded, onCancel }: VoiceRecorderProps) {
  const [recording, setRecording] = useState(false);
  const [duration, setDuration] = useState(0);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const timerRef = useRef<ReturnType<typeof setInterval> | undefined>(undefined);
  const streamRef = useRef<MediaStream | null>(null);

  const cleanup = useCallback(() => {
    clearInterval(timerRef.current);
    if (streamRef.current) {
      streamRef.current.getTracks().forEach((t) => t.stop());
      streamRef.current = null;
    }
    setRecording(false);
  }, []);

  const startRecording = useCallback(async () => {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;
      const mediaRecorder = new MediaRecorder(stream, {
        mimeType: MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
          ? 'audio/webm;codecs=opus'
          : 'audio/webm',
      });
      mediaRecorderRef.current = mediaRecorder;
      chunksRef.current = [];

      mediaRecorder.ondataavailable = (e) => {
        if (e.data.size > 0) {
          chunksRef.current.push(e.data);
        }
      };

      mediaRecorder.onstop = () => {
        const blob = new Blob(chunksRef.current, { type: mediaRecorder.mimeType });
        onRecorded(blob);
        cleanup();
      };

      mediaRecorder.start(100); // Collect data every 100ms
      setRecording(true);
      setDuration(0);
      timerRef.current = setInterval(() => {
        setDuration((d) => d + 1);
      }, 1000);
    } catch (err) {
      console.error('Failed to start recording:', err);
      onCancel();
    }
  }, [onRecorded, onCancel, cleanup]);

  const stopRecording = useCallback(() => {
    if (mediaRecorderRef.current && mediaRecorderRef.current.state !== 'inactive') {
      mediaRecorderRef.current.stop();
    }
  }, []);

  const handleCancel = useCallback(() => {
    cleanup();
    onCancel();
  }, [cleanup, onCancel]);

  // Start immediately on mount — setState in startRecording is after async getUserMedia
  useEffect(() => {
    startRecording(); // eslint-disable-line react-hooks/set-state-in-effect
    return () => {
      cleanup();
    };
  }, [startRecording, cleanup]);

  const formatTime = (seconds: number) => {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${s.toString().padStart(2, '0')}`;
  };

  return (
    <div className="flex items-center gap-3 rounded-lg bg-bg-input px-4 py-3">
      {/* Pulsing red dot */}
      <span className="relative flex h-3 w-3">
        <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-red-400 opacity-75" />
        <span className="relative inline-flex h-3 w-3 rounded-full bg-red-500" />
      </span>

      <span className="text-sm text-text-secondary">
        {recording ? `Recording ${formatTime(duration)}` : 'Starting...'}
      </span>

      <div className="flex-1" />

      <button
        onClick={handleCancel}
        className="rounded p-1.5 text-text-muted transition-colors hover:text-red-400"
        title="Cancel recording"
      >
        <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>

      {recording && (
        <button
          onClick={stopRecording}
          className="rounded bg-blue-600 p-1.5 text-white transition-colors hover:bg-blue-500"
          title="Stop and send"
        >
          <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
            <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z" />
          </svg>
        </button>
      )}
    </div>
  );
}
