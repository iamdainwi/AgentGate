import type { InvocationRecord, OverviewStats, ToolStat } from './types'
import { AuthError, clearToken, getToken } from './auth'

const API_BASE = process.env.NEXT_PUBLIC_API_BASE ?? 'http://localhost:7070'

export const WS_BASE = API_BASE.replace(/^http/, 'ws')

/** Build a WebSocket URL with the auth token as a query parameter.
 *  Browsers do not support custom headers on WebSocket connections,
 *  so the token is passed via ?token=. */
export function wsLiveUrl(): string {
  const token = getToken()
  const qs = token ? `?token=${encodeURIComponent(token)}` : ''
  return `${WS_BASE}/api/ws/live${qs}`
}

function authHeaders(): HeadersInit {
  const token = getToken()
  return token ? { Authorization: `Bearer ${token}` } : {}
}

async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: { ...authHeaders(), ...init?.headers },
  })

  if (res.status === 401) {
    clearToken()
    throw new AuthError()
  }

  if (!res.ok) {
    const err = new Error(`HTTP ${res.status}`) as Error & { status: number }
    err.status = res.status
    throw err
  }

  return res.json() as Promise<T>
}

export async function getInvocations(params: {
  limit?: number
  offset?: number
  tool?: string
  status?: string
}): Promise<InvocationRecord[]> {
  const qs = new URLSearchParams()
  if (params.limit != null) qs.set('limit', String(params.limit))
  if (params.offset != null) qs.set('offset', String(params.offset))
  if (params.tool) qs.set('tool', params.tool)
  if (params.status) qs.set('status', params.status)
  const query = qs.toString() ? `?${qs}` : ''
  return fetchJson<InvocationRecord[]>(`/api/invocations${query}`)
}

export async function getInvocation(id: string): Promise<InvocationRecord> {
  return fetchJson<InvocationRecord>(`/api/invocations/${id}`)
}

export async function getOverviewStats(): Promise<OverviewStats> {
  return fetchJson<OverviewStats>('/api/stats/overview')
}

export async function getToolStats(): Promise<ToolStat[]> {
  return fetchJson<ToolStat[]>('/api/stats/tools')
}

export async function getPolicies(): Promise<string> {
  const res = await fetch(`${API_BASE}/api/policies`, { headers: authHeaders() })
  if (res.status === 401) {
    clearToken()
    throw new AuthError()
  }
  if (res.status === 404) return ''
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return res.text()
}

export async function putPolicies(body: string): Promise<{ ok: boolean }> {
  const res = await fetch(`${API_BASE}/api/policies`, {
    method: 'PUT',
    headers: { 'Content-Type': 'text/plain', ...authHeaders() },
    body,
  })
  if (res.status === 401) {
    clearToken()
    throw new AuthError()
  }
  return { ok: res.ok }
}

/** Probe the API with the given token. Returns true if the server accepts it. */
export async function verifyToken(token: string): Promise<boolean> {
  const res = await fetch(`${API_BASE}/api/stats/overview`, {
    headers: { Authorization: `Bearer ${token}` },
  })
  return res.status !== 401
}
