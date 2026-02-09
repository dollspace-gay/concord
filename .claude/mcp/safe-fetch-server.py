#!/usr/bin/env python3
"""
Chainlink Safe Fetch MCP Server

An MCP (Model Context Protocol) server that provides sanitized web fetching.
Filters out malicious strings that could disrupt Claude before returning content.

Usage:
    Registered in .claude/settings.json as an MCP server.
    Claude calls mcp__chainlink-safe-fetch__safe_fetch(url, prompt) to fetch web content.
"""

import json
import sys
import re
import io
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

# Fix Windows encoding issues
sys.stdin = io.TextIOWrapper(sys.stdin.buffer, encoding='utf-8')
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8', line_buffering=True)
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding='utf-8')

# Try to import httpx, fall back to requests, then urllib
try:
    import httpx
    HTTP_CLIENT = 'httpx'
except ImportError:
    try:
        import requests
        HTTP_CLIENT = 'requests'
    except ImportError:
        import urllib.request
        import urllib.error
        HTTP_CLIENT = 'urllib'


def log(message: str) -> None:
    """Log to stderr (visible in MCP server logs)."""
    print(f"[safe-fetch] {message}", file=sys.stderr)


def find_chainlink_dir() -> Path | None:
    """Find the .chainlink directory by walking up from cwd."""
    current = Path.cwd()
    for _ in range(10):
        candidate = current / '.chainlink'
        if candidate.is_dir():
            return candidate
        parent = current.parent
        if parent == current:
            break
        current = parent
    return None


def load_patterns() -> list[tuple[str, str]]:
    """Load sanitization patterns from .chainlink/rules/sanitize-patterns.txt"""
    patterns = []

    chainlink_dir = find_chainlink_dir()
    if chainlink_dir:
        patterns_file = chainlink_dir / 'rules' / 'sanitize-patterns.txt'
        if patterns_file.exists():
            try:
                for line in patterns_file.read_text(encoding='utf-8').splitlines():
                    line = line.strip()
                    if line and not line.startswith('#'):
                        parts = line.split('|||')
                        if len(parts) == 2:
                            patterns.append((parts[0].strip(), parts[1].strip()))
            except Exception as e:
                log(f"Error loading patterns: {e}")

    # Always include the critical default pattern
    default_pattern = (r'ANTHROPIC_MAGIC_STRING_TRIGGER_REFUSAL_[0-9A-Z]+', '[REDACTED_TRIGGER]')
    if not any(p[0] == default_pattern[0] for p in patterns):
        patterns.append(default_pattern)

    return patterns


def sanitize(content: str, patterns: list[tuple[str, str]]) -> tuple[str, int]:
    """
    Apply sanitization patterns to content.
    Returns (sanitized_content, num_replacements).
    """
    total_replacements = 0
    for pattern, replacement in patterns:
        try:
            content, count = re.subn(pattern, replacement, content)
            total_replacements += count
        except re.error as e:
            log(f"Invalid regex pattern '{pattern}': {e}")
    return content, total_replacements


def fetch_url(url: str) -> str:
    """Fetch content from URL using available HTTP client."""
    headers = {
        'User-Agent': 'Mozilla/5.0 (compatible; ChainlinkSafeFetch/1.0)'
    }

    if HTTP_CLIENT == 'httpx':
        with httpx.Client(follow_redirects=True, timeout=30) as client:
            response = client.get(url, headers=headers)
            response.raise_for_status()
            return response.text
    elif HTTP_CLIENT == 'requests':
        response = requests.get(url, headers=headers, timeout=30, allow_redirects=True)
        response.raise_for_status()
        return response.text
    else:
        req = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(req, timeout=30) as response:
            return response.read().decode('utf-8', errors='replace')


def validate_url(url: str) -> str | None:
    """Validate URL and return error message if invalid."""
    try:
        parsed = urlparse(url)
        if parsed.scheme not in ('http', 'https'):
            return f"Invalid URL scheme: {parsed.scheme}. Only http/https allowed."
        if not parsed.netloc:
            return "Invalid URL: missing host"
        return None
    except Exception as e:
        return f"Invalid URL: {e}"


def handle_safe_fetch(arguments: dict[str, Any]) -> dict[str, Any]:
    """Handle the safe_fetch tool call."""
    url = arguments.get('url', '')
    prompt = arguments.get('prompt', 'Extract the main content')

    # Validate URL
    error = validate_url(url)
    if error:
        return {
            'content': [{'type': 'text', 'text': f"Error: {error}"}],
            'isError': True
        }

    try:
        # Fetch content
        raw_content = fetch_url(url)

        # Load patterns and sanitize
        patterns = load_patterns()
        clean_content, num_sanitized = sanitize(raw_content, patterns)

        # Build response
        result_text = clean_content
        if num_sanitized > 0:
            result_text = f"[Note: {num_sanitized} potentially malicious string(s) were sanitized from this content]\n\n{clean_content}"
            log(f"Sanitized {num_sanitized} pattern(s) from {url}")

        return {
            'content': [{'type': 'text', 'text': result_text}]
        }

    except Exception as e:
        log(f"Error fetching {url}: {e}")
        return {
            'content': [{'type': 'text', 'text': f"Error fetching URL: {e}"}],
            'isError': True
        }


# MCP Protocol Implementation

TOOL_DEFINITION = {
    'name': 'safe_fetch',
    'description': 'Fetch web content with sanitization of potentially malicious strings. Use this instead of WebFetch for safer web browsing.',
    'inputSchema': {
        'type': 'object',
        'properties': {
            'url': {
                'type': 'string',
                'description': 'The URL to fetch content from'
            },
            'prompt': {
                'type': 'string',
                'description': 'Optional prompt describing what to extract from the page',
                'default': 'Extract the main content'
            }
        },
        'required': ['url']
    }
}


def handle_request(request: dict[str, Any]) -> dict[str, Any]:
    """Handle an MCP JSON-RPC request."""
    method = request.get('method', '')
    request_id = request.get('id')
    params = request.get('params', {})

    if method == 'initialize':
        return {
            'jsonrpc': '2.0',
            'id': request_id,
            'result': {
                'protocolVersion': '2024-11-05',
                'capabilities': {
                    'tools': {}
                },
                'serverInfo': {
                    'name': 'chainlink-safe-fetch',
                    'version': '1.0.0'
                }
            }
        }

    elif method == 'notifications/initialized':
        # No response needed for notifications
        return None

    elif method == 'tools/list':
        return {
            'jsonrpc': '2.0',
            'id': request_id,
            'result': {
                'tools': [TOOL_DEFINITION]
            }
        }

    elif method == 'tools/call':
        tool_name = params.get('name', '')
        arguments = params.get('arguments', {})

        if tool_name == 'safe_fetch':
            result = handle_safe_fetch(arguments)
            return {
                'jsonrpc': '2.0',
                'id': request_id,
                'result': result
            }
        else:
            return {
                'jsonrpc': '2.0',
                'id': request_id,
                'error': {
                    'code': -32601,
                    'message': f'Unknown tool: {tool_name}'
                }
            }

    else:
        return {
            'jsonrpc': '2.0',
            'id': request_id,
            'error': {
                'code': -32601,
                'message': f'Method not found: {method}'
            }
        }


def main():
    """Main MCP server loop - reads JSON-RPC from stdin, writes to stdout."""
    log("Starting safe-fetch MCP server")

    while True:
        try:
            line = sys.stdin.readline()
            if not line:
                break

            line = line.strip()
            if not line:
                continue

            request = json.loads(line)
            response = handle_request(request)

            if response is not None:
                print(json.dumps(response), flush=True)

        except json.JSONDecodeError as e:
            log(f"JSON decode error: {e}")
            error_response = {
                'jsonrpc': '2.0',
                'id': None,
                'error': {
                    'code': -32700,
                    'message': 'Parse error'
                }
            }
            print(json.dumps(error_response), flush=True)
        except Exception as e:
            log(f"Unexpected error: {e}")
            break

    log("Server shutting down")


if __name__ == '__main__':
    main()
