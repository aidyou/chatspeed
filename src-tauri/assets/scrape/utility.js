function sendEvent(event, data) {
  if (window.__TAURI__?.event) {
    try {
      window.__TAURI__.event.emit(event, data)
    } catch (error) {
      console.error('Failed to send scrape result:', error)
    }
  } else {
    console.warn('window.__TAURI__.event not available, data not sent: ', data)
  }
}

const getTimestamp = () => {
  const now = new Date()
  const timeStr = now.toTimeString().slice(0, 8) // HH:MM:SS
  const ms = now.getMilliseconds().toString().padStart(3, '0') // .000~.999
  return `${timeStr}.${ms}`
}

const logger = {
  debug: (...args) => {
    console.debug(`${getTimestamp()} [D]`, ...args)
    sendEvent('logger_event', { window: windowLabel || '', message: args.join(' ') })
  },
  info: (...args) => {
    console.info(`${getTimestamp()} [I]`, ...args)
    sendEvent('logger_event', { window: windowLabel || '', message: args.join(' ') })
  },
  warn: (...args) => {
    console.warn(`${getTimestamp()} [W]`, ...args)
    sendEvent('logger_event', { window: windowLabel || '', message: args.join(' ') })
  },
  error: (...args) => {
    console.error(`${getTimestamp()} [E]`, ...args)
    sendEvent('logger_event', { window: windowLabel || '', message: args.join(' ') })
  }
}

const formatText = txt => {
  return txt == null
    ? txt
    : txt
        .toString()
        .replace(/\n{3,}/g, '\n\n')
        .trim()
}

const baseCleanForMarkdown = el => {
  if (!el) return el

  el.querySelectorAll(
    'script, style, noscript, form, iframe, frame, object, ' +
      'embed, video, audio, link, svg, canvas, meta, head, ' +
      'base, template, symbol, button, select, textarea, ' +
      'datalist, dialog, source, picture, track, map'
  ).forEach(el => el.remove())

  return el
}
