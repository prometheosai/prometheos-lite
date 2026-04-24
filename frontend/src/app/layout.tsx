import type { Metadata } from 'next'
import './globals.css'

export const metadata: Metadata = {
  title: 'PrometheOS Lite',
  description: 'Local AI-powered flow execution interface',
}

export default function RootLayout({
  children,
}: {
  children: React.ReactNode
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  )
}
