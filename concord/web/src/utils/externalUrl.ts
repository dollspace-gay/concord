const MAX_EXTERNAL_URL_LENGTH = 2_048;
const CANONICAL_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

function isPrivateIpv4(hostname: string): boolean {
  const parts = hostname.split('.').map(Number);
  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) return false;
  return parts[0] === 10
    || parts[0] === 127
    || parts[0] === 0
    || (parts[0] === 169 && parts[1] === 254)
    || (parts[0] === 172 && parts[1] >= 16 && parts[1] <= 31)
    || (parts[0] === 192 && parts[1] === 168)
    || parts[0] >= 224;
}

function isObviouslyPrivateHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, '');
  const isPrivateIpv6 = host.includes(':') && (
    host === '::1'
    || host.startsWith('fc')
    || host.startsWith('fd')
    || host.startsWith('fe8')
    || host.startsWith('fe9')
    || host.startsWith('fea')
    || host.startsWith('feb')
    || (host.startsWith('::ffff:') && isPrivateIpv4(host.slice('::ffff:'.length)))
  );
  return host === 'localhost'
    || host.endsWith('.localhost')
    || host.endsWith('.local')
    || host.endsWith('.internal')
    || isPrivateIpv6
    || isPrivateIpv4(host);
}

/** Browser-side defense for externally supplied destinations. */
export function safeExternalHttpsUrl(value?: string | null): string | undefined {
  if (!value || value.length > MAX_EXTERNAL_URL_LENGTH) return undefined;
  try {
    const parsed = new URL(value);
    if (parsed.protocol !== 'https:'
      || !parsed.hostname
      || parsed.username
      || parsed.password
      || isObviouslyPrivateHost(parsed.hostname)) return undefined;
    return parsed.toString();
  } catch {
    return undefined;
  }
}

/** Accepts only the relative URL shape emitted for authenticated private media. */
export function safeSameOriginUploadUrl(value?: string | null): string | undefined {
  if (!value) return undefined;
  const prefix = '/api/uploads/';
  if (!value.startsWith(prefix)) return undefined;
  return CANONICAL_UUID.test(value.slice(prefix.length)) ? value : undefined;
}
