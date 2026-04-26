"use client"

import { useState } from "react"
import { GitBranch, PanelLeft, PanelRight, FileText, MessageSquare, Play, CheckCircle, AlertCircle, Clock, Cpu, Database, Plus, X, Check, Bug, Pause, SkipForward, Square, RotateCcw, Shield, AlertTriangle } from "lucide-react"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"

interface FlowEvent {
  type: 'node_start' | 'node_end' | 'output' | 'error'
  data: {
    node?: string
    timestamp: string
    data?: string
    message?: string
    model?: string
  }
}

interface RightSidebarProps {
  events: FlowEvent[]
  status: string
}

export function RightSidebar({ events, status }: RightSidebarProps) {
  const [collapsed, setCollapsed] = useState(false)
  const [activeTab, setActiveTab] = useState<"flow" | "artifacts" | "memory" | "debug" | "policy">("flow")
  const [memorySubTab, setMemorySubTab] = useState<"retrieved" | "new" | "pending" | "approved">("retrieved")
  const [flowSubTab, setFlowSubTab] = useState<"timeline" | "models">("timeline")
  const [debugMode, setDebugMode] = useState(false)

  const getEventIcon = (event: FlowEvent) => {
    switch (event.type) {
      case 'node_start':
        return <Play className="h-3 w-3" />
      case 'node_end':
        return <CheckCircle className="h-3 w-3" />
      case 'output':
        return <Cpu className="h-3 w-3" />
      case 'error':
        return <AlertCircle className="h-3 w-3" />
      default:
        return null
    }
  }

  const getEventColor = (event: FlowEvent) => {
    switch (event.type) {
      case 'error':
        return 'bg-red-900/20 border-red-800/50 text-red-200'
      case 'output':
        return 'bg-green-900/20 border-green-800/50 text-green-200'
      case 'node_start':
        return 'bg-blue-900/20 border-blue-800/50 text-blue-200'
      case 'node_end':
        return 'bg-purple-900/20 border-purple-800/50 text-purple-200'
      default:
        return 'bg-muted border-border'
    }
  }

  const getNodeLabel = (node?: string) => {
    if (!node) return 'Unknown'
    const labels: Record<string, string> = {
      'planner': 'Orchestrating',
      'coder': 'Executing',
      'reviewer': 'Validating',
      'memory_write': 'Integrating',
      'system': 'Thinking',
    }
    return labels[node] || node.charAt(0).toUpperCase() + node.slice(1)
  }

  return (
    <TooltipProvider delayDuration={0}>
      <div
        className={cn(
          "flex flex-col transition-all duration-200 ease-linear h-full overflow-hidden",
          collapsed ? "w-16 bg-background" : "w-80 bg-sidebar border-l border-border"
        )}
      >
        {/* Header */}
        <div className={cn("flex h-16 items-center justify-between px-4", !collapsed && "border-b border-border")}>
          {!collapsed && (
            <div className="flex items-center gap-1">
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    onClick={() => setActiveTab("flow")}
                    className={cn(
                      "flex items-center justify-center w-9 h-9 rounded-md text-sm transition-colors",
                      activeTab === "flow"
                        ? "bg-sidebar-accent text-sidebar-accent-foreground"
                        : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                    )}
                  >
                    <GitBranch className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom">Flow</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    onClick={() => setActiveTab("memory")}
                    className={cn(
                      "flex items-center justify-center w-9 h-9 rounded-md text-sm transition-colors",
                      activeTab === "memory"
                        ? "bg-sidebar-accent text-sidebar-accent-foreground"
                        : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                    )}
                  >
                    <Database className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom">Memory</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    onClick={() => setActiveTab("policy")}
                    className={cn(
                      "flex items-center justify-center w-9 h-9 rounded-md text-sm transition-colors",
                      activeTab === "policy"
                        ? "bg-sidebar-accent text-sidebar-accent-foreground"
                        : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                    )}
                  >
                    <Shield className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom">Policy</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    onClick={() => {
                      setActiveTab("debug")
                      setDebugMode(!debugMode)
                    }}
                    className={cn(
                      "flex items-center justify-center w-9 h-9 rounded-md text-sm transition-colors",
                      activeTab === "debug"
                        ? "bg-sidebar-accent text-sidebar-accent-foreground"
                        : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                    )}
                  >
                    <Bug className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom">Debug</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    onClick={() => setActiveTab("artifacts")}
                    className={cn(
                      "flex items-center justify-center w-9 h-9 rounded-md text-sm transition-colors",
                      activeTab === "artifacts"
                        ? "bg-sidebar-accent text-sidebar-accent-foreground"
                        : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                    )}
                  >
                    <FileText className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom">Artifacts</TooltipContent>
              </Tooltip>
            </div>
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                onClick={() => setCollapsed(!collapsed)}
                className="ml-auto"
              >
                {collapsed ? <PanelLeft className="h-4 w-4" /> : <PanelRight className="h-4 w-4" />}
              </Button>
            </TooltipTrigger>
            <TooltipContent side="left">
              {collapsed ? "Expand" : "Collapse"}
            </TooltipContent>
          </Tooltip>
        </div>

        {/* Content Area */}
        {!collapsed && (
          <ScrollArea className="flex-1">
            <div className="p-3 space-y-2">
              {activeTab === "flow" ? (
                <>
                  <div className="mb-3 pb-2 border-b border-border">
                    <div className="text-xs font-mono-label text-sidebar-foreground">
                      Status: <span className="text-sidebar-accent-foreground">{status}</span>
                    </div>
                  </div>
                  <Tabs value={flowSubTab} onValueChange={(v) => setFlowSubTab(v as any)}>
                    <TabsList className="w-full grid grid-cols-2 h-8">
                      <TabsTrigger value="timeline" className="text-xs h-7">Timeline</TabsTrigger>
                      <TabsTrigger value="models" className="text-xs h-7">Models</TabsTrigger>
                    </TabsList>
                    <TabsContent value="timeline" className="mt-2 space-y-2">
                      {events.length === 0 ? (
                        <div className="text-center text-sm text-muted-foreground py-8">
                          No events yet
                        </div>
                      ) : (
                        <div className="space-y-2">
                          {events.map((event, index) => {
                            const prevEvent = index > 0 ? events[index - 1] : null
                            const startTime = prevEvent ? new Date(prevEvent.data.timestamp) : null
                            const endTime = new Date(event.data.timestamp)
                            const duration = startTime ? (endTime.getTime() - startTime.getTime()) / 1000 : null
                            
                            return (
                              <div
                                key={index}
                                className={cn(
                                  "relative pl-5 pb-2 message-animate transition-all duration-200",
                                  index < events.length - 1 && "border-l-2 border-border/30 ml-2"
                                )}
                              >
                                <div className="absolute left-0 top-0 flex items-center justify-center w-3.5 h-3.5 rounded-full bg-background border-2 border-border">
                                  {getEventIcon(event)}
                                </div>
                                <div
                                  className={cn(
                                    "p-2 rounded-lg text-xs border transition-all duration-200 hover-glow",
                                    getEventColor(event)
                                  )}
                                >
                                  <div className="flex items-center justify-between gap-2">
                                    <div className="flex items-center gap-1.5 font-medium">
                                      {event.type === 'node_start' && (
                                        <>{getNodeLabel(event.data.node)}…</>
                                      )}
                                      {event.type === 'node_end' && (
                                        <>Completed {getNodeLabel(event.data.node)}</>
                                      )}
                                      {event.type === 'output' && (
                                        <>Output: {getNodeLabel(event.data.node)}</>
                                      )}
                                      {event.type === 'error' && (
                                        <>Error: {getNodeLabel(event.data.node)}</>
                                      )}
                                    </div>
                                    {duration !== null && duration > 0 && (
                                      <div className="flex items-center gap-1 text-[10px] opacity-70">
                                        <Clock className="h-2.5 w-2.5" />
                                        {duration.toFixed(1)}s
                                      </div>
                                    )}
                                  </div>
                                  {event.data.model && (
                                    <div className="mt-1 text-[10px] opacity-75 flex items-center gap-1">
                                      <Cpu className="h-2.5 w-2.5" />
                                      {event.data.model}
                                    </div>
                                  )}
                                  {event.data.data && (
                                    <div className="mt-1 text-[10px] opacity-90 line-clamp-2">
                                      {event.data.data}
                                    </div>
                                  )}
                                  {event.data.message && (
                                    <div className="mt-1 text-[10px] text-red-300">
                                      {event.data.message}
                                    </div>
                                  )}
                                  <div className="text-[10px] mt-1 opacity-60 font-mono-label">
                                    {new Date(event.data.timestamp).toLocaleTimeString()}
                                  </div>
                                </div>
                              </div>
                            )
                          })}
                        </div>
                      )}
                    </TabsContent>
                    <TabsContent value="models" className="mt-2 space-y-2">
                      <div className="space-y-2">
                        <div className="p-2 rounded-lg bg-muted/50 border">
                          <div className="flex items-center justify-between mb-1">
                            <div className="text-[10px] font-medium">Planner</div>
                            <div className="text-[10px] text-muted-foreground">LM Studio</div>
                          </div>
                          <div className="text-xs">google/gemma-4-e4b</div>
                          <div className="text-[10px] text-muted-foreground mt-0.5">Local • 4B parameters</div>
                        </div>
                        <div className="p-2 rounded-lg bg-muted/50 border">
                          <div className="flex items-center justify-between mb-1">
                            <div className="text-[10px] font-medium">Coder</div>
                            <div className="text-[10px] text-muted-foreground">LM Studio</div>
                          </div>
                          <div className="text-xs">google/gemma-4-e4b</div>
                          <div className="text-[10px] text-muted-foreground mt-0.5">Local • 4B parameters</div>
                        </div>
                        <div className="p-2 rounded-lg bg-muted/50 border">
                          <div className="flex items-center justify-between mb-1">
                            <div className="text-[10px] font-medium">Reviewer</div>
                            <div className="text-[10px] text-muted-foreground">LM Studio</div>
                          </div>
                          <div className="text-xs">google/gemma-4-e4b</div>
                          <div className="text-[10px] text-muted-foreground mt-0.5">Local • 4B parameters</div>
                        </div>
                        <div className="p-2 rounded-lg bg-muted/50 border">
                          <div className="flex items-center justify-between mb-1">
                            <div className="text-[10px] font-medium">Memory</div>
                            <div className="text-[10px] text-muted-foreground">Local</div>
                          </div>
                          <div className="text-xs">Embedding (1536 dim)</div>
                          <div className="text-[10px] text-muted-foreground mt-0.5">LM Studio • text-embedding-ada-002</div>
                        </div>
                        <div className="p-2 rounded-lg bg-muted/50 border">
                          <div className="flex items-center justify-between mb-1">
                            <div className="text-[10px] font-medium">Estimated Cost</div>
                            <div className="text-[10px] text-muted-foreground">Session</div>
                          </div>
                          <div className="text-xs">$0.00</div>
                          <div className="text-[10px] text-muted-foreground mt-0.5">All models running locally</div>
                        </div>
                      </div>
                    </TabsContent>
                  </Tabs>
                </>
              ) : activeTab === "memory" ? (
                <div className="space-y-2">
                  <Tabs value={memorySubTab} onValueChange={(v) => setMemorySubTab(v as any)}>
                    <TabsList className="w-full grid grid-cols-4 h-8">
                      <TabsTrigger value="retrieved" className="text-[10px] h-7">Retrieved</TabsTrigger>
                      <TabsTrigger value="new" className="text-[10px] h-7">New</TabsTrigger>
                      <TabsTrigger value="pending" className="text-[10px] h-7">Pending</TabsTrigger>
                      <TabsTrigger value="approved" className="text-[10px] h-7">History</TabsTrigger>
                    </TabsList>
                    <TabsContent value="retrieved" className="mt-2">
                      <div className="text-xs text-muted-foreground text-center py-4">
                        No retrieved context yet
                      </div>
                    </TabsContent>
                    <TabsContent value="new" className="mt-2">
                      <div className="text-xs text-muted-foreground text-center py-4">
                        No new memories
                      </div>
                    </TabsContent>
                    <TabsContent value="pending" className="mt-2">
                      <div className="text-xs text-muted-foreground text-center py-4">
                        No pending memory writes
                      </div>
                    </TabsContent>
                    <TabsContent value="approved" className="mt-2">
                      <div className="text-xs text-muted-foreground text-center py-4">
                        No memory history
                      </div>
                    </TabsContent>
                  </Tabs>
                </div>
              ) : activeTab === "policy" ? (
                <div className="space-y-2">
                  <div className="p-2 rounded-lg bg-muted/50 border">
                    <div className="flex items-center gap-1.5 mb-1">
                      <Shield className="h-3 w-3" />
                      <div className="text-[10px] font-medium">Sandbox Profile</div>
                    </div>
                    <div className="text-xs">Standard</div>
                    <div className="text-[10px] text-muted-foreground mt-0.5">Filesystem: Workspace only • Network: Local only • Shell: Restricted</div>
                  </div>

                  <div className="p-2 rounded-lg bg-muted/50 border">
                    <div className="flex items-center gap-1.5 mb-1">
                      <CheckCircle className="h-3 w-3 text-green-500" />
                      <div className="text-[10px] font-medium">Capability Checks</div>
                    </div>
                    <div className="space-y-1.5 mt-2">
                      <div className="flex items-center justify-between text-[10px]">
                        <span>File Read</span>
                        <span className="text-green-500">Allowed</span>
                      </div>
                      <div className="flex items-center justify-between text-[10px]">
                        <span>File Write</span>
                        <span className="text-green-500">Allowed</span>
                      </div>
                      <div className="flex items-center justify-between text-[10px]">
                        <span>Shell Execution</span>
                        <span className="text-yellow-500">Restricted</span>
                      </div>
                      <div className="flex items-center justify-between text-[10px]">
                        <span>Network Access</span>
                        <span className="text-red-500">Blocked</span>
                      </div>
                    </div>
                  </div>

                  <div className="p-2 rounded-lg bg-muted/50 border">
                    <div className="flex items-center gap-1.5 mb-1">
                      <AlertTriangle className="h-3 w-3 text-yellow-500" />
                      <div className="text-[10px] font-medium">Policy Violations</div>
                    </div>
                    <div className="text-xs text-muted-foreground text-center py-4">
                      No violations in current session
                    </div>
                  </div>

                  <div className="p-2 rounded-lg bg-muted/50 border">
                    <div className="flex items-center gap-1.5 mb-1">
                      <AlertCircle className="h-3 w-3 text-red-500" />
                      <div className="text-[10px] font-medium">Blocked Actions</div>
                    </div>
                    <div className="text-xs text-muted-foreground text-center py-4">
                      No blocked actions in current session
                    </div>
                  </div>
                </div>
              ) : activeTab === "debug" ? (
                <div className="space-y-2">
                  <div className="flex items-center justify-between mb-2">
                    <p className="text-xs font-medium">Debug State Inspector</p>
                    <div className="flex gap-0.5">
                      <Button size="sm" variant="ghost" className="h-6 px-1.5">
                        <RotateCcw className="h-2.5 w-2.5" />
                      </Button>
                      <Button size="sm" variant="ghost" className="h-6 px-1.5">
                        <Pause className="h-2.5 w-2.5" />
                      </Button>
                      <Button size="sm" variant="ghost" className="h-6 px-1.5">
                        <SkipForward className="h-2.5 w-2.5" />
                      </Button>
                      <Button size="sm" variant="ghost" className="h-6 px-1.5 text-red-500">
                        <Square className="h-2.5 w-2.5" />
                      </Button>
                    </div>
                  </div>
                  
                  <div className="space-y-2">
                    <div className="p-2 rounded-lg bg-muted/50 border">
                      <div className="text-[10px] font-medium mb-1 text-muted-foreground">Current Node</div>
                      <div className="text-xs">Not executing</div>
                    </div>
                    
                    <div className="p-2 rounded-lg bg-muted/50 border">
                      <div className="text-[10px] font-medium mb-1 text-muted-foreground">Input State</div>
                      <pre className="text-[10px] overflow-x-auto">{JSON.stringify({ status: "idle" }, null, 2)}</pre>
                    </div>
                    
                    <div className="p-2 rounded-lg bg-muted/50 border">
                      <div className="text-[10px] font-medium mb-1 text-muted-foreground">Output State</div>
                      <pre className="text-[10px] overflow-x-auto">{JSON.stringify({}, null, 2)}</pre>
                    </div>
                    
                    <div className="p-2 rounded-lg bg-muted/50 border">
                      <div className="text-[10px] font-medium mb-1 text-muted-foreground">Next Transition</div>
                      <div className="text-xs">None</div>
                    </div>
                  </div>
                </div>
              ) : (
                <div className="text-xs text-muted-foreground text-center py-8">
                  Generated files will appear here
                </div>
              )}
            </div>
          </ScrollArea>
        )}
      </div>
    </TooltipProvider>
  )
}
