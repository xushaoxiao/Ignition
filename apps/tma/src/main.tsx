import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import GamesPreview from './GamesPreview'
import '@ignition/games/styles.css'
import './styles.css'

// `/?preview` renders the dev game gallery (no backend/Telegram needed); everything else is the app.
const preview = new URLSearchParams(window.location.search).has('preview')

createRoot(document.getElementById('root')!).render(
  <StrictMode>{preview ? <GamesPreview /> : <App />}</StrictMode>,
)
