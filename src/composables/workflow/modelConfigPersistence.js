export function createLatestModelConfigSaver({ save, onSuccess, onFailure }) {
  let pendingSubmission = null
  let isSaving = false
  let activeScope = null
  let activeActivation = 0

  const invalidateScope = scope => {
    activeScope = scope
    activeActivation += 1
    pendingSubmission = null
    return activeActivation
  }

  const submit = async submission => {
    if (activeScope !== submission.scope) {
      const activation = invalidateScope(submission.scope)
      submission = { ...submission, activation }
    } else if (submission.activation === undefined) {
      submission = { ...submission, activation: activeActivation }
    } else if (activeActivation !== submission.activation) {
      return
    }
    pendingSubmission = submission
    if (isSaving) return

    isSaving = true
    try {
      while (pendingSubmission) {
        const nextSubmission = pendingSubmission
        pendingSubmission = null
        let saved = false
        try {
          saved = await save(nextSubmission)
        } catch (error) {
          console.error('Failed to save latest model config:', error)
        }

        const isCurrentActivation =
          activeScope === nextSubmission.scope && activeActivation === nextSubmission.activation
        const hasNewerCurrentSubmission =
          pendingSubmission &&
          pendingSubmission.scope === activeScope &&
          pendingSubmission.activation === activeActivation
        if (!saved && isCurrentActivation && !hasNewerCurrentSubmission) {
          onFailure?.(nextSubmission)
          break
        }

        if (saved && isCurrentActivation && !pendingSubmission) {
          onSuccess?.(nextSubmission)
        }
      }
    } finally {
      isSaving = false
    }
  }

  return {
    invalidateScope,
    submit,
    isSaving: () => isSaving
  }
}
