import type { Metadata } from 'next'
import './globals.css'
import { ThemeProvider } from '@/components/theme-provider'
import { ChatProvider } from '@/context/chat-context'
import { ProfileProvider } from '@/context/profile-context'

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
    <html lang="en" suppressHydrationWarning>
      <body suppressHydrationWarning>
        <ThemeProvider
          attribute="class"
          defaultTheme="dark"
          enableSystem
          disableTransitionOnChange
        >
          <ProfileProvider>
            <ChatProvider>
              {children}
            </ChatProvider>
          </ProfileProvider>
        </ThemeProvider>
      </body>
    </html>
  )
}
