"use client"

import { useState } from "react"
import { LeftSidebar } from "./left-sidebar"
import { RightSidebar } from "./right-sidebar"
import { ConversationArea } from "./conversation-area"
import { useTheme } from "next-themes"
import { Moon, Sun, ChevronDown, Cpu } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { type FlowEvent } from "@/lib/api"
import { useChat } from "@/context/chat-context"

interface AppLayoutProps {
  children?: React.ReactNode
  hideConversation?: boolean
}

const modelProviders = [
  { key: "lmstudio", label: "LM Studio", type: "local" },
  { key: "llama", label: "Llama", type: "local" },
  { key: "openai", label: "OpenAI", type: "cloud" },
  { key: "anthropic", label: "Anthropic", type: "cloud" },
  { key: "groq", label: "Groq", type: "cloud" },
] as const

export function AppLayout({ children, hideConversation = false }: AppLayoutProps) {
  const { theme, setTheme } = useTheme()
  const { currentConversation } = useChat()
  const [events, setEvents] = useState<FlowEvent[]>([])
  const [status, setStatus] = useState("Idle")
  const [selectedProvider, setSelectedProvider] = useState("lmstudio")

  const handleFlowEvent = (event: FlowEvent) => {
    setEvents((prev) => [...prev, event])
    
    if (event.type === 'node_start') {
      setStatus(`Running: ${event.data.node}`)
    } else if (event.type === 'node_end') {
      setStatus("Processing...")
    } else if (event.type === 'error') {
      setStatus("Error")
    } else if (event.type === 'output' && event.data.node === 'system') {
      setStatus(event.data.data || "Processing...")
    }
  }

  const activeProvider = modelProviders.find(p => p.key === selectedProvider) || modelProviders[0]

  return (
    <div className="flex h-screen w-full bg-background overflow-hidden">
      {/* Left Sidebar - Hidden on mobile, visible on md and up */}
      <div className="hidden md:block">
        <LeftSidebar />
      </div>

      {/* Main Content */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Top Bar */}
        <div className="h-16 flex items-center justify-between px-4 md:px-6 bg-background flex-shrink-0">
          <div className="flex items-center gap-4">
            {currentConversation ? (
              <h1 className="text-base md:text-lg font-display font-semibold text-foreground">
                {currentConversation.title}
              </h1>
            ) : (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors">
                    <Cpu className="h-4 w-4" />
                    <span className="hidden sm:inline">{activeProvider.label}</span>
                    <ChevronDown className="h-3 w-3" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  {modelProviders.map((provider) => (
                    <DropdownMenuItem
                      key={provider.key}
                      onClick={() => setSelectedProvider(provider.key)}
                      className="flex items-center justify-between gap-4"
                    >
                      <span>{provider.label}</span>
                      <span className="text-xs text-muted-foreground capitalize">{provider.type}</span>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            )}
          </div>
          <TooltipProvider delayDuration={0}>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
                  className="hidden sm:flex"
                >
                  {theme === "dark" ? (
                    <Sun className="h-4 w-4" />
                  ) : (
                    <Moon className="h-4 w-4" />
                  )}
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </div>

        {/* Content Area */}
        <div className="flex-1 min-w-0 overflow-hidden">
          {children || (!hideConversation && <ConversationArea onFlowEvent={handleFlowEvent} />)}
        </div>
      </div>

      {/* Right Sidebar - Hidden on mobile, visible on lg and up */}
      <div className="hidden lg:block">
        <RightSidebar events={events} status={status} />
      </div>
    </div>
  )
}
