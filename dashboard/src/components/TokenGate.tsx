'use client'

import { createContext, useCallback, useContext, useEffect, useState } from 'react'
import { AuthError, clearToken, getToken, setToken } from '@/lib/auth'
import { verifyToken } from '@/lib/api'

// ── Context ───────────────────────────────────────────────────────────────────

interface AuthCtx {
  /** Call this when any API response returns 401 to bring up the modal. */
  onUnauthorized: () => void
}

const AuthContext = createContext<AuthCtx>({ onUnauthorized: () => {} })

export function useAuth(): AuthCtx {
  return useContext(AuthContext)
}

// ── Modal ─────────────────────────────────────────────────────────────────────

function TokenModal({ onSuccess }: { onSuccess: () => void }) {
  const [value, setValue] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [checking, setChecking] = useState(false)

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    const trimmed = value.trim()
    if (!trimmed) return

    setChecking(true)
    setError(null)

    try {
      const ok = await verifyToken(trimmed)
      if (ok) {
        setToken(trimmed)
        onSuccess()
      } else {
        setError('Token rejected — check the value printed in the AgentGate terminal output.')
      }
    } catch {
      setError('Could not reach AgentGate. Is it running on port 7070?')
    } finally {
      setChecking(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-gray-950/90 backdrop-blur-sm">
      <div className="w-full max-w-md rounded-2xl border border-gray-700 bg-gray-900 p-8 shadow-2xl">
        <div className="mb-6 flex items-center gap-3">
          {/* Shield icon */}
          <span className="flex h-10 w-10 items-center justify-center rounded-full bg-indigo-600/20 text-indigo-400">
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" className="h-5 w-5">
              <path
                fillRule="evenodd"
                d="M12 1.5a5.25 5.25 0 00-5.25 5.25v3a3 3 0 00-3 3v6.75a3 3 0 003 3h10.5a3 3 0 003-3v-6.75a3 3 0 00-3-3v-3A5.25 5.25 0 0012 1.5zm3.75 8.25v-3a3.75 3.75 0 10-7.5 0v3h7.5z"
                clipRule="evenodd"
              />
            </svg>
          </span>
          <div>
            <h1 className="text-lg font-semibold text-white">AgentGate Dashboard</h1>
            <p className="text-sm text-gray-400">Enter your API token to continue</p>
          </div>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label htmlFor="token" className="mb-1.5 block text-xs font-medium text-gray-400 uppercase tracking-wider">
              API Token
            </label>
            <input
              id="token"
              type="password"
              autoFocus
              autoComplete="off"
              spellCheck={false}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder="Paste the token printed by AgentGate…"
              className="w-full rounded-lg border border-gray-700 bg-gray-800 px-4 py-2.5 font-mono text-sm text-gray-100 placeholder-gray-600 outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
            />
          </div>

          {error && (
            <p className="rounded-md bg-red-900/40 border border-red-700/50 px-3 py-2 text-sm text-red-300">
              {error}
            </p>
          )}

          <button
            type="submit"
            disabled={checking || !value.trim()}
            className="w-full rounded-lg bg-indigo-600 py-2.5 text-sm font-medium text-white transition-colors hover:bg-indigo-500 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {checking ? 'Verifying…' : 'Unlock Dashboard'}
          </button>
        </form>

        <p className="mt-5 text-xs text-gray-600">
          The token is printed to the terminal when AgentGate starts. You can also set{' '}
          <code className="text-gray-500">dashboard_api_key</code> in{' '}
          <code className="text-gray-500">~/.agentgate/config.toml</code> to persist it.
        </p>
      </div>
    </div>
  )
}

// ── Gate ──────────────────────────────────────────────────────────────────────

export default function TokenGate({ children }: { children: React.ReactNode }) {
  const [locked, setLocked] = useState(false)

  // On mount, check if we already have a stored token.
  // If not, show the modal immediately.
  useEffect(() => {
    if (!getToken()) {
      setLocked(true)
    }
  }, [])

  const onUnauthorized = useCallback(() => {
    clearToken()
    setLocked(true)
  }, [])

  return (
    <AuthContext.Provider value={{ onUnauthorized }}>
      {locked && <TokenModal onSuccess={() => setLocked(false)} />}
      {children}
    </AuthContext.Provider>
  )
}
