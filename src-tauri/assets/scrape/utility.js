function sendEvent(event,data) {
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
    sendEvent('logger', {message: args.join(' ') })
  },
  info: (...args) => {
    console.info(`${getTimestamp()} [I]`, ...args)
    sendEvent('logger', {message: args.join(' ') })
  },
  warn: (...args) => {
    console.warn(`${getTimestamp()} [W]`, ...args)
    sendEvent('logger', {message: args.join(' ') })
  },
  error: (...args) => {
    console.error(`${getTimestamp()} [E]`, ...args)
    sendEvent('logger', {message: args.join(' ') })
  }
}
