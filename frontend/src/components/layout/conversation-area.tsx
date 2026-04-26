"use client"

import { useState, useRef, useEffect } from "react"
import { Send, Loader2, ChevronDown, ChevronUp, Edit2, RefreshCw, Brain, User } from "lucide-react"
import { Hero } from "@/components/hero"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { ScrollArea } from "@/components/ui/scroll-area"
import { useChat } from "@/context/chat-context"
import { runFlow, connectWebSocket, type FlowEvent } from "@/lib/api"
import { cn } from "@/lib/utils"
import { getVerbForNode, getErrorVerb } from "@/lib/verbs"
import ReactMarkdown from 'react-markdown'

export function ConversationArea({ onFlowEvent }: { onFlowEvent?: (event: FlowEvent) => void }) {
  const { currentConversation, addMessage, setCurrentConversation } = useChat()
  const [input, setInput] = useState("")
  const [running, setRunning] = useState(false)
  const [thinking, setThinking] = useState(false)
  const [currentVerb, setCurrentVerb] = useState<string | null>(null)
  const [flowEvents, setFlowEvents] = useState<FlowEvent[]>([])
  const [editingPlan, setEditingPlan] = useState(false)
  const [editedPlanContent, setEditedPlanContent] = useState("")
  const messagesEndRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    scrollToBottom()
  }, [currentConversation?.messages])

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" })
  }

  const handleSendMessage = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!input.trim() || !currentConversation || running) return

    const userMessage = input
    setInput("")
    setRunning(true)
    setThinking(true)
    setFlowEvents([]) // Reset flow events for new message

    // Add user message
    addMessage(currentConversation.id, { role: "user", content: userMessage })

    // Timeout to reset running state if flow doesn't complete
    const timeoutId = setTimeout(() => {
      setRunning(false)
      setThinking(false)
    }, 60000) // 60 second timeout

    try {
      // Run flow via API
      const flowRun = await runFlow(currentConversation.id, userMessage)
      
      // Connect to WebSocket for real-time events
      const ws = connectWebSocket(flowRun.id, (event) => {
        if (onFlowEvent) {
          onFlowEvent(event)
        }
        
        // Track flow events for display in chat area
        setFlowEvents(prev => [...prev, event])
        
        if (event.type === 'node_start') {
          // Set verb based on node name and ensure thinking state is true
          const verb = getVerbForNode(event.data.node || "system")
          if (verb) {
            setCurrentVerb(verb)
            setThinking(true)
          }
        } else if (event.type === 'node_end') {
          // Reset verb when node completes
          setCurrentVerb(null)
          // Check if this is the system node completing (flow finished)
          if (event.data.node === 'system') {
            setRunning(false)
            setThinking(false)
            setFlowEvents([]) // Clear flow events when flow completes
            clearTimeout(timeoutId)
            ws.close()
          }
        } else if (event.type === 'output' && event.data.node === 'assistant') {
          // Assistant response
          setThinking(false)
          setCurrentVerb(null)
          setFlowEvents([]) // Clear flow events when response received
          addMessage(currentConversation.id, {
            role: "assistant",
            content: event.data.data || "",
          })
        } else if (event.type === 'error') {
          setThinking(false)
          setRunning(false)
          setCurrentVerb(getErrorVerb())
          setFlowEvents([]) // Clear flow events on error
          clearTimeout(timeoutId)
          addMessage(currentConversation.id, {
            role: "assistant",
            content: `Error: ${event.data.message}`,
          })
        }
      })

      ws.onerror = () => {
        console.error("WebSocket error")
        setThinking(false)
        setRunning(false)
        clearTimeout(timeoutId)
        addMessage(currentConversation.id, {
          role: "assistant",
          content: "Connection error. Please try again.",
        })
      }

      ws.onclose = () => {
        // Ensure running state is reset when connection closes
        setRunning(false)
        setThinking(false)
        clearTimeout(timeoutId)
      }

    } catch (error) {
      console.error("Failed to run flow:", error)
      setThinking(false)
      setRunning(false)
      clearTimeout(timeoutId)
      addMessage(currentConversation.id, {
        role: "assistant",
        content: "Failed to process your request. Please check if the backend server is running.",
      })
    }
  }

  if (!currentConversation) {
    return (
      <div className="h-full bg-background">
        <Hero onFlowEvent={onFlowEvent} />
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full bg-background min-w-0">
      {/* Chat Area */}
      <ScrollArea className="flex-1">
        <div className="max-w-4xl mx-auto px-4 py-6 space-y-4">
          {currentConversation?.messages.length === 0 && (
            <div className="text-center py-12">
              <div className="text-4xl mb-4">💬</div>
              <h3 className="text-lg font-semibold mb-2">Start a conversation</h3>
              <p className="text-sm text-muted-foreground">Ask anything about your projects or code</p>
            </div>
          )}
          
          {currentConversation?.messages.map((message) => (
            <div
              key={message.id}
              className={cn(
                "flex gap-3 animate-in fade-in slide-in-from-bottom-2 duration-300",
                message.role === "user" ? "justify-end" : "justify-start"
              )}
            >
              {message.role === "assistant" && (
                <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                  <Brain className="h-4 w-4 text-primary" />
                </div>
              )}
              <div
                className={cn(
                  "max-w-2xl rounded-lg px-4 py-3",
                  message.role === "user"
                    ? "bg-primary text-primary-foreground"
                    : "bg-muted border border-border"
                )}
              >
                {message.role === "assistant" ? (
                  <div className="prose prose-sm dark:prose-invert max-w-none">
                    <ReactMarkdown>
                      {message.content}
                    </ReactMarkdown>
                  </div>
                ) : (
                  <div className="text-sm">{message.content}</div>
                )}
              </div>
              {message.role === "user" && (
                <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/20 flex items-center justify-center">
                  <User className="h-4 w-4 text-primary-foreground" />
                </div>
              )}
            </div>
          ))}
          {thinking && (
            <div className="flex justify-start">
              <div className="flex items-center gap-2 text-muted-foreground text-xs">
                <Loader2 className="animate-spin h-3 w-3" />
                <span className="transition-opacity duration-300">
                  {currentVerb ? `${currentVerb}…` : "Thinking…"}
                </span>
              </div>
            </div>
          )}
          {flowEvents.length > 0 && thinking && (
            <div className="flex justify-start">
              <div className="max-w-2xl px-3 py-2 rounded-lg bg-muted/50 text-muted-foreground text-xs space-y-1">
                {flowEvents.slice(-5).map((event, idx) => (
                  <div key={idx} className="flex items-center gap-2">
                    {event.type === 'node_start' && (
                      <span className="text-primary">▶ {event.data.node}</span>
                    )}
                    {event.type === 'node_end' && (
                      <span className="text-green-600">✓ {event.data.node}</span>
                    )}
                    {event.type === 'output' && (
                      <span className="text-blue-600">→ Output: {event.data.node}</span>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}
          {(() => {
            const lastEvent = flowEvents.length > 0 ? flowEvents[flowEvents.length - 1] : null;
            const isWaitingForApproval = lastEvent?.type === 'output' && 
                                        lastEvent?.data?.data && 
                                        lastEvent.data.data.includes('waiting for approval');
            const lastAssistantMessage = currentConversation?.messages.filter(m => m.role === 'assistant').slice(-1)[0];
            
            if (!thinking && isWaitingForApproval) {
              return (
                <div className="flex justify-start">
                  <div className="max-w-2xl px-4 py-4 rounded-lg bg-muted/50 border border-border">
                    <div className="flex items-center justify-between mb-3">
                      <p className="text-sm font-medium">Plan generated. Would you like to proceed with implementation?</p>
                      <div className="flex gap-1">
                        <Button
                          onClick={() => setEditingPlan(!editingPlan)}
                          size="sm"
                          variant="ghost"
                          className="h-7 px-2"
                        >
                          <Edit2 className="h-3 w-3" />
                        </Button>
                        <Button
                          onClick={() => {
                            setInput("Regenerate the plan")
                            const formEvent = new Event('submit') as any
                            formEvent.preventDefault = () => {}
                            handleSendMessage(formEvent)
                          }}
                          size="sm"
                          variant="ghost"
                          className="h-7 px-2"
                        >
                          <RefreshCw className="h-3 w-3" />
                        </Button>
                      </div>
                    </div>
                    
                    {editingPlan && lastAssistantMessage ? (
                      <Textarea
                        value={editedPlanContent || lastAssistantMessage.content}
                        onChange={(e) => setEditedPlanContent(e.target.value)}
                        className="mb-3 min-h-[150px] text-xs"
                        placeholder="Edit the plan before approval..."
                      />
                    ) : (
                      <div className="mb-3 p-3 rounded bg-background/50 text-xs max-h-[150px] overflow-y-auto">
                        {lastAssistantMessage ? lastAssistantMessage.content.substring(0, 300) + (lastAssistantMessage.content.length > 300 ? '...' : '') : 'No plan content'}
                      </div>
                    )}
                    
                    <div className="flex gap-2">
                      <Button
                        onClick={() => {
                          setInput(editingPlan && editedPlanContent ? `Implement this plan: ${editedPlanContent}` : "Implement this plan")
                          setEditingPlan(false)
                          setEditedPlanContent("")
                          const formEvent = new Event('submit') as any
                          formEvent.preventDefault = () => {}
                          handleSendMessage(formEvent)
                        }}
                        size="sm"
                        className="bg-primary text-primary-foreground"
                      >
                        Implement Plan
                      </Button>
                      <Button
                        onClick={() => {
                          setInput(editingPlan && editedPlanContent ? `Modify the plan: ${editedPlanContent}` : "Modify the plan")
                          setEditingPlan(false)
                          setEditedPlanContent("")
                          setRunning(false)
                          setThinking(false)
                        }}
                        size="sm"
                        variant="outline"
                      >
                        Modify Plan
                      </Button>
                      {editingPlan && (
                        <Button
                          onClick={() => {
                            setEditingPlan(false)
                            setEditedPlanContent("")
                          }}
                          size="sm"
                          variant="ghost"
                        >
                          Cancel
                        </Button>
                      )}
                    </div>
                  </div>
                </div>
              )
            }
            return null;
          })()}
          <div ref={messagesEndRef} />
        </div>
      </ScrollArea>

      {/* Input */}
      <div className="p-4 border-t border-border flex-shrink-0">
        <form onSubmit={handleSendMessage} className="flex gap-2 max-w-3xl mx-auto">
          <Input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Type your message..."
            disabled={running}
            className="flex-1"
          />
          <Button
            type="submit"
            disabled={running || !input.trim()}
            size="icon"
          >
            {running ? (
              <Loader2 className="animate-spin h-4 w-4" />
            ) : (
              <Send className="h-4 w-4" />
            )}
          </Button>
        </form>
      </div>
    </div>
  )
}
