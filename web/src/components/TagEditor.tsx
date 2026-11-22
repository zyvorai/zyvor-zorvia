// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { useState } from 'react'
import { X, Plus } from 'lucide-react'
import { updateTags } from '../api/vm'
import { useToastContext } from '../contexts/ToastContext'
import { toastFailure } from '../utils/toastError'

interface TagEditorProps {
  vmName: string
  currentTags: string[]
  onClose: () => void
  onSuccess: () => void
}

const NEUTRAL_TAG = 'bg-[var(--zf-fill-tertiary)] text-[var(--zf-ink)] border border-[var(--zf-hairline)]'

const TAG_COLORS: Record<string, string> = {
  production: 'zf-tint-danger border',
  staging: 'zf-tint-warning border',
  development: 'zf-tint-success border',
  testing: 'zf-tint-info border',
  default: NEUTRAL_TAG,
}

export function getTagColor(tag: string): string {
  const normalizedTag = tag.toLowerCase()
  return TAG_COLORS[normalizedTag] || TAG_COLORS.default
}

export default function TagEditor({ vmName, currentTags, onClose, onSuccess }: TagEditorProps) {
  const toast = useToastContext()
  const [tags, setTags] = useState<string[]>(currentTags || [])
  const [newTag, setNewTag] = useState('')
  const [saving, setSaving] = useState(false)

  const handleAddTag = () => {
    const trimmedTag = newTag.trim()
    if (trimmedTag && !tags.includes(trimmedTag)) {
      setTags([...tags, trimmedTag])
      setNewTag('')
    }
  }

  const handleRemoveTag = (tag: string) => {
    setTags(tags.filter((t) => t !== tag))
  }

  const handleSave = async () => {
    setSaving(true)
    try {
      await updateTags(vmName, tags)
      toast.success('Tags updated successfully')
      onSuccess()
      onClose()
    } catch (error) {
      toastFailure(toast, 'Failed to update tags', error)
    } finally {
      setSaving(false)
    }
  }

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault()
      handleAddTag()
    }
  }

  const commonTags = ['production', 'staging', 'development', 'testing', 'web', 'database', 'backend', 'frontend']
  const suggestedTags = commonTags.filter(tag => !tags.includes(tag))

  return (
    <div className="bg-[var(--zf-surface)] border border-[var(--zf-hairline)] rounded-xl p-5 space-y-4">
      <h3 className="text-sm font-semibold text-[var(--zf-ink)]">Manage Tags · {vmName}</h3>

      <div>
        <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Current Tags</label>
        {tags.length === 0 ? (
          <p className="text-sm text-[var(--zf-muted)]">No tags assigned</p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {tags.map((tag) => (
              <span
                key={tag}
                className={`inline-flex items-center gap-2 px-3 py-1.5 rounded-full text-sm font-medium ${getTagColor(tag)}`}
              >
                {tag}
                <button
                  onClick={() => handleRemoveTag(tag)}
                  className="hover:bg-[var(--zf-active-tint)] rounded-full p-0.5 transition"
                >
                  <X className="w-3.5 h-3.5" />
                </button>
              </span>
            ))}
          </div>
        )}
      </div>

      <div>
        <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Add New Tag</label>
        <div className="flex gap-2">
          <input
            type="text"
            value={newTag}
            onChange={(e) => setNewTag(e.target.value)}
            onKeyDown={handleKeyPress}
            placeholder="Enter tag name..."
            className="input-field text-sm flex-1"
          />
          <button
            onClick={handleAddTag}
            disabled={!newTag.trim() || tags.includes(newTag.trim())}
            className="zf-btn zf-btn-ghost zf-btn-sm"
          >
            <Plus className="w-4 h-4" />
            Add
          </button>
        </div>
      </div>

      {suggestedTags.length > 0 && (
        <div>
          <label className="block text-xs font-medium text-[var(--zf-muted)] mb-1.5">Suggested Tags</label>
          <div className="flex flex-wrap gap-2">
            {suggestedTags.map((tag) => (
              <button
                key={tag}
                onClick={() => setTags([...tags, tag])}
                className={`px-3 py-1.5 rounded-full text-sm font-medium ${getTagColor(tag)} hover:opacity-80 transition`}
              >
                + {tag}
              </button>
            ))}
          </div>
        </div>
      )}

      <div className="flex gap-2">
        <button
          onClick={() => void handleSave()}
          disabled={saving}
          className="zf-btn zf-btn-primary zf-btn-sm"
        >
          {saving ? 'Saving...' : 'Save Tags'}
        </button>
        <button
          onClick={onClose}
          disabled={saving}
          className="zf-btn zf-btn-ghost zf-btn-sm"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}
