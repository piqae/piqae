import type { Access, FieldAccess } from 'payload'

type Role = 'admin' | 'editor' | 'reviewer'

function roleOf(user: unknown): Role | null {
  if (!user || typeof user !== 'object') return null
  const role = (user as { role?: unknown }).role
  return role === 'admin' || role === 'editor' || role === 'reviewer' ? role : null
}

export const anyone: Access = () => true
export const authenticated: Access = ({ req }) => Boolean(req.user)
export const adminOnly: Access = ({ req }) => roleOf(req.user) === 'admin'
export const canEditContent: Access = ({ req }) => {
  const role = roleOf(req.user)
  return role === 'admin' || role === 'editor'
}
export const canReviewContent: Access = ({ req }) => {
  const role = roleOf(req.user)
  return role === 'admin' || role === 'reviewer'
}
export const canEditOrReviewContent: Access = ({ req }) => {
  const role = roleOf(req.user)
  return role === 'admin' || role === 'editor' || role === 'reviewer'
}
export const publishedOrAuthenticated: Access = ({ req }) =>
  req.user ? true : { _status: { equals: 'published' } }

export const adminFieldOnly: FieldAccess = ({ req }) => roleOf(req.user) === 'admin'
export const reviewerFieldOnly: FieldAccess = ({ req }) => {
  const role = roleOf(req.user)
  return role === 'admin' || role === 'reviewer'
}
