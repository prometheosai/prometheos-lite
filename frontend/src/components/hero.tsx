"use client"

import { useState, useEffect } from "react"
import { Send, Plus, Wrench, Mic, AudioLines, Image as ImageIcon, FileText, Link2 } from "lucide-react"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogTrigger } from "@/components/ui/dialog"
import { useChat } from "@/context/chat-context"
import { runFlow, connectWebSocket, type FlowEvent } from "@/lib/api"
import { getVerbForNode, getErrorVerb } from "@/lib/verbs"

interface HeroProps {
  onFlowEvent?: (event: FlowEvent) => void
}

export function Hero({ onFlowEvent }: HeroProps) {
  const [message, setMessage] = useState("")
  const [isLoading, setIsLoading] = useState(false)
  const [uploadOpen, setUploadOpen] = useState(false)
  const [toolsOpen, setToolsOpen] = useState(false)
  const [currentVerb, setCurrentVerb] = useState<string | null>(null)
  const { createConversation, addMessage, setCurrentConversation, currentConversationId } = useChat()

  const [starter] = useState("Hey. Ready to dive in?")

  const handleSubmit = async () => {
    if (!message.trim()) return
    const text = message.trim()
    setIsLoading(true)
    try {
      // Create conversation and get ID
      const conversationId = await createConversation({ title: text.slice(0, 40) })
      
      // Add user message and run flow
      addMessage(conversationId, { role: "user", content: text })
      
      // Run flow
      const flowRun = await runFlow(conversationId, text)
      
      // Connect to WebSocket for real-time events
      const ws = connectWebSocket(flowRun.id, (event) => {
        if (onFlowEvent) {
          onFlowEvent(event)
        }
        
        if (event.type === 'node_start') {
          // Set verb based on node name and ensure loading state is true
          const verb = getVerbForNode(event.data.node || "system")
          if (verb) {
            setCurrentVerb(verb)
            setIsLoading(true)
          }
        } else if (event.type === 'node_end') {
          // Reset verb when node completes
          setCurrentVerb(null)
          // Check if this is the system node completing (flow finished)
          if (event.data.node === 'system') {
            setIsLoading(false)
            ws.close()
          }
        } else if (event.type === 'output' && event.data.node === 'assistant') {
          addMessage(conversationId, {
            role: "assistant",
            content: event.data.data || "",
          })
          setCurrentVerb(null)
        } else if (event.type === 'error') {
          setCurrentVerb(getErrorVerb())
          addMessage(conversationId, {
            role: "assistant",
            content: `Error: ${event.data.message}`,
          })
        }
      })

      ws.onerror = () => {
        setIsLoading(false)
        addMessage(conversationId, {
          role: "assistant",
          content: "Connection error. Please try again.",
        })
      }
    } catch (err) {
      setIsLoading(false)
    } finally {
      setMessage("")
    }
  }

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      handleSubmit()
    }
  }

  return (
    <div className="flex flex-col items-center p-6 bg-background h-screen">
      <div className="flex-1"></div>
      <div className="w-full max-w-[600px] mx-auto mb-[40vh]">
        <div className="flex justify-center mb-12">
          <p className="text-2xl md:text-3xl font-medium text-center text-foreground/90">{starter}</p>
        </div>

        <div className="relative">
          <div className="flex flex-col gap-3 p-4 bg-background border rounded-[10px] shadow-sm focus-within:border-primary transition-colors">
            <textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              onKeyDown={handleKeyPress}
              placeholder="Ask anything"
              rows={1}
              disabled={isLoading}
              className="w-full resize-none bg-transparent text-foreground placeholder:text-muted-foreground focus:outline-none leading-relaxed max-h-48 overflow-y-auto"
            />
            <div className="flex items-end justify-between">
              <div className="flex items-center gap-2">
                <Dialog open={uploadOpen} onOpenChange={setUploadOpen}>
                  <DialogTrigger asChild>
                    <button onClick={() => setUploadOpen(true)} className="p-2 hover:bg-muted rounded-lg transition-colors" aria-label="Attach files">
                      <Plus className="h-5 w-5 text-muted-foreground" />
                    </button>
                  </DialogTrigger>
                  <DialogContent className="sm:max-w-md">
                    <DialogHeader>
                      <DialogTitle>Attach to message</DialogTitle>
                      <DialogDescription>Upload files or add links for context.</DialogDescription>
                    </DialogHeader>
                    <div className="grid grid-cols-3 gap-3">
                      <button className="flex flex-col items-center gap-2 p-3 rounded-[10px] border hover:bg-muted transition-colors" aria-label="Upload image">
                        <ImageIcon className="h-5 w-5" /><span className="text-xs">Image</span>
                      </button>
                      <button className="flex flex-col items-center gap-2 p-3 rounded-[10px] border hover:bg-muted transition-colors" aria-label="Upload document">
                        <FileText className="h-5 w-5" /><span className="text-xs">Document</span>
                      </button>
                      <button className="flex flex-col items-center gap-2 p-3 rounded-[10px] border hover:bg-muted transition-colors" aria-label="Add link">
                        <Link2 className="h-5 w-5" /><span className="text-xs">Link</span>
                      </button>
                    </div>
                  </DialogContent>
                </Dialog>
                <Dialog open={toolsOpen} onOpenChange={setToolsOpen}>
                  <DialogTrigger asChild>
                    <button onClick={() => setToolsOpen(true)} disabled={isLoading} className="flex items-center gap-2 px-3 py-2 rounded-lg text-sm hover:bg-muted text-muted-foreground transition-colors">
                      <Wrench className="h-4 w-4" /><span>Tools</span>
                    </button>
                  </DialogTrigger>
                  <DialogContent className="sm:max-w-md">
                    <DialogHeader>
                      <DialogTitle>Tools</DialogTitle>
                      <DialogDescription>Choose utilities to enhance your prompt.</DialogDescription>
                    </DialogHeader>
                    <div className="grid grid-cols-2 gap-3">
                      <button className="p-3 rounded-[10px] border hover:bg-muted transition-colors text-sm">Summarize</button>
                      <button className="p-3 rounded-[10px] border hover:bg-muted transition-colors text-sm">Translate</button>
                      <button className="p-3 rounded-[10px] border hover:bg-muted transition-colors text-sm">Code</button>
                      <button className="p-3 rounded-[10px] border hover:bg-muted transition-colors text-sm">Brainstorm</button>
                    </div>
                  </DialogContent>
                </Dialog>
              </div>
              <div className="flex items-center gap-2">
                <button type="button" onClick={() => {}} disabled={isLoading} className="p-2 hover:bg-muted rounded-lg transition-colors" title="Start recording">
                  <Mic className="h-5 w-5" />
                </button>
                <button disabled={isLoading} className="p-2 hover:bg-muted rounded-lg transition-colors" title="Start live AI call">
                  <AudioLines className="h-5 w-5 text-muted-foreground" />
                </button>
                {message.trim() && (
                  <button onClick={handleSubmit} disabled={isLoading} className="p-2 bg-primary hover:bg-primary/90 text-primary-foreground rounded-lg transition-colors">
                    <Send className="h-4 w-4" />
                  </button>
                )}
              </div>
            </div>
          </div>
        </div>

        {isLoading && (
          <div className="mt-6 text-center">
            <div className="inline-flex items-center gap-2 text-muted-foreground text-xs">
              <div className="w-3 h-3 border-2 border-primary border-t-transparent rounded-full animate-spin"></div>
              <span className="transition-opacity duration-300">
                {currentVerb ? `${currentVerb}…` : "Creating conversation…"}
              </span>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
