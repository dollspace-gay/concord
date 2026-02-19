import { useState, useEffect, useRef, useCallback } from 'react';

const TENOR_API_KEY = import.meta.env.VITE_TENOR_API_KEY as string | undefined;
const TENOR_BASE = 'https://tenor.googleapis.com/v2';

interface TenorGif {
  id: string;
  title: string;
  media_formats: {
    tinygif?: { url: string; dims: [number, number] };
    gif?: { url: string };
  };
}

interface GifPickerProps {
  onSelect: (url: string) => void;
  onClose: () => void;
}

// eslint-disable-next-line react-refresh/only-export-components
export function isGifPickerAvailable(): boolean {
  return !!TENOR_API_KEY;
}

export function GifPicker({ onSelect, onClose }: GifPickerProps) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<TenorGif[]>([]);
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const fetchGifs = useCallback(async (searchQuery: string) => {
    if (!TENOR_API_KEY) return;
    setLoading(true);
    try {
      const endpoint = searchQuery.trim()
        ? `${TENOR_BASE}/search?q=${encodeURIComponent(searchQuery)}&key=${TENOR_API_KEY}&limit=20&media_filter=tinygif,gif`
        : `${TENOR_BASE}/featured?key=${TENOR_API_KEY}&limit=20&media_filter=tinygif,gif`;
      const res = await fetch(endpoint);
      if (res.ok) {
        const data = await res.json();
        setResults(data.results || []);
      }
    } catch (err) {
      console.error('Tenor search failed:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  // Load featured GIFs on mount
  useEffect(() => {
    fetchGifs('');
    inputRef.current?.focus();
  }, [fetchGifs]);

  // Debounced search
  useEffect(() => {
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      fetchGifs(query);
    }, 400);
    return () => clearTimeout(debounceRef.current);
  }, [query, fetchGifs]);

  const handleSelect = (gif: TenorGif) => {
    const url = gif.media_formats.gif?.url || gif.media_formats.tinygif?.url;
    if (url) {
      onSelect(url);
    }
  };

  return (
    <div className="absolute bottom-full left-0 right-0 mb-1 flex max-h-80 flex-col overflow-hidden rounded-lg border border-border bg-bg-secondary shadow-lg">
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search GIFs..."
          className="flex-1 bg-transparent text-sm text-text-primary placeholder-text-muted outline-none"
          onKeyDown={(e) => {
            if (e.key === 'Escape') onClose();
          }}
        />
        <button onClick={onClose} className="text-text-muted hover:text-text-primary">
          <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
      <div className="flex-1 overflow-y-auto p-2">
        {loading && results.length === 0 ? (
          <div className="flex items-center justify-center py-8 text-sm text-text-muted">Loading...</div>
        ) : results.length === 0 ? (
          <div className="flex items-center justify-center py-8 text-sm text-text-muted">No GIFs found</div>
        ) : (
          <div className="grid grid-cols-3 gap-1">
            {results.map((gif) => {
              const preview = gif.media_formats.tinygif;
              if (!preview) return null;
              return (
                <button
                  key={gif.id}
                  onClick={() => handleSelect(gif)}
                  className="overflow-hidden rounded transition-opacity hover:opacity-80"
                  title={gif.title}
                >
                  <img
                    src={preview.url}
                    alt={gif.title}
                    className="h-24 w-full object-cover"
                    loading="lazy"
                  />
                </button>
              );
            })}
          </div>
        )}
      </div>
      <div className="border-t border-border px-3 py-1 text-right text-[10px] text-text-muted">
        Powered by Tenor
      </div>
    </div>
  );
}
