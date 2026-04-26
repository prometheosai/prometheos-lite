"use client"

import { useState, useMemo, useRef, useEffect } from "react"
import {
  MessageSquare,
  Folder,
  Plus,
  ChevronRight,
  ChevronLeft,
  Settings,
  User,
  Brain,
  Code2,
  FileText,
  Lightbulb,
  Rocket,
  Star,
  PanelLeft,
  PanelRight,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { Separator } from "@/components/ui/separator"
import { useChat } from "@/context/chat-context"
import { cn } from "@/lib/utils"

const chatIconOptions = [
  { key: "message-square", label: "Chat", Icon: MessageSquare },
  { key: "brain", label: "Brainstorm", Icon: Brain },
  { key: "code-2", label: "Code", Icon: Code2 },
  { key: "file-text", label: "Docs", Icon: FileText },
  { key: "lightbulb", label: "Idea", Icon: Lightbulb },
  { key: "rocket", label: "Launch", Icon: Rocket },
  { key: "star", label: "Starred", Icon: Star },
] as const

const projectIconOptions = [
  { key: "folder", label: "Folder", Icon: Folder },
  { key: "lightbulb", label: "Idea", Icon: Lightbulb },
  { key: "rocket", label: "Launch", Icon: Rocket },
  { key: "star", label: "Starred", Icon: Star },
  { key: "code-2", label: "Code", Icon: Code2 },
  { key: "brain", label: "Brainstorm", Icon: Brain },
] as const

export function LeftSidebar() {
  const [collapsed, setCollapsed] = useState(false)
  const [selectedProjectIcon, setSelectedProjectIcon] = useState("folder")
  const [selectedChatIcon, setSelectedChatIcon] = useState("message-square")
  const [editingProjectId, setEditingProjectId] = useState<string | null>(null)
  const [editingConversationId, setEditingConversationId] = useState<string | null>(null)
  const { projects, conversations, currentConversationId, createProject, createConversation, setCurrentConversation, setProjectIcon, setConversationIcon, renameProject, renameConversation, deleteProject, archiveProject, duplicateProject, deleteConversation, archiveConversation, duplicateConversation } = useChat()

  const iconMap: Record<string, any> = {
    "folder": Folder,
    "message-square": MessageSquare,
    "brain": Brain,
    "code-2": Code2,
    "file-text": FileText,
    "lightbulb": Lightbulb,
    "rocket": Rocket,
    "star": Star,
  }

  const RenderIcon = ({ name, className }: { name: string; className?: string }) => {
    const Icon = iconMap[name] || MessageSquare
    return <Icon className={className} />
  }

  const EditableText = ({ 
    value, 
    isEditing, 
    onSave, 
    onCancel, 
    onStartEdit,
    onEdit,
    onDelete,
    onArchive,
    onDuplicate,
    onAddChat,
    className
  }: {
    value: string
    isEditing: boolean
    onSave: (newValue: string) => void
    onCancel: () => void
    onStartEdit: (e?: React.MouseEvent) => void
    onEdit?: () => void
    onDelete?: () => void
    onArchive?: () => void
    onDuplicate?: () => void
    onAddChat?: () => void
    className?: string
  }) => {
    const [editValue, setEditValue] = useState(value)
    const [contextMenuOpen, setContextMenuOpen] = useState(false)
    const inputRef = useRef<HTMLInputElement>(null)
    const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(null)
    const menuRef = useRef<HTMLDivElement>(null)

    useEffect(() => {
      setEditValue(value)
    }, [value])

    useEffect(() => {
      if (isEditing && inputRef.current) {
        inputRef.current.focus()
        inputRef.current.select()
      }
    }, [isEditing])

    useEffect(() => {
      if (contextMenuOpen) {
        const handleClick = () => setContextMenuOpen(false)
        const handleEscape = (e: KeyboardEvent) => {
          if (e.key === "Escape") setContextMenuOpen(false)
        }
        document.addEventListener("click", handleClick)
        document.addEventListener("keydown", handleEscape)
        return () => {
          document.removeEventListener("click", handleClick)
          document.removeEventListener("keydown", handleEscape)
        }
      }
    }, [contextMenuOpen])

    if (isEditing) {
      return (
        <input
          ref={inputRef}
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          onBlur={() => {
            if (editValue.trim()) {
              onSave(editValue.trim())
            } else {
              onCancel()
            }
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault()
              if (editValue.trim()) {
                onSave(editValue.trim())
              } else {
                onCancel()
              }
            } else if (e.key === "Escape") {
              onCancel()
            }
          }}
          className={cn("bg-background border border-border rounded px-1 py-0.5 text-sm w-full focus:outline-none focus:ring-2 focus:ring-primary", className)}
          onClick={(e) => e.stopPropagation()}
        />
      )
    }

    return (
      <span
        className={cn("cursor-pointer hover:bg-sidebar-accent/50 rounded px-1 py-0.5 transition-colors relative", className)}
        onDoubleClick={(e) => {
          e.stopPropagation()
          onStartEdit(e)
        }}
        onContextMenu={(e) => {
          e.preventDefault()
          e.stopPropagation()
          setMenuPos({ x: e.clientX, y: e.clientY })
          setContextMenuOpen(true)
        }}
      >
        {value}
        {contextMenuOpen && menuPos && (
          <div
            ref={menuRef}
            className="fixed z-50 min-w-[160px] overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
            style={{ left: menuPos.x, top: menuPos.y }}
            onClick={(e) => e.stopPropagation()}
          >
            {onAddChat && (
              <button
                className="relative flex w-full cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none transition-colors hover:bg-accent hover:text-accent-foreground"
                onClick={() => {
                  onAddChat()
                  setContextMenuOpen(false)
                }}
              >
                New Chat
              </button>
            )}
            {onEdit && (
              <button
                className="relative flex w-full cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none transition-colors hover:bg-accent hover:text-accent-foreground"
                onClick={() => {
                  onEdit()
                  setContextMenuOpen(false)
                }}
              >
                Edit
              </button>
            )}
            {onDuplicate && (
              <button
                className="relative flex w-full cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none transition-colors hover:bg-accent hover:text-accent-foreground"
                onClick={() => {
                  onDuplicate()
                  setContextMenuOpen(false)
                }}
              >
                Duplicate
              </button>
            )}
            {onArchive && (
              <button
                className="relative flex w-full cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none transition-colors hover:bg-accent hover:text-accent-foreground"
                onClick={() => {
                  onArchive()
                  setContextMenuOpen(false)
                }}
              >
                Archive
              </button>
            )}
            {onDelete && (
              <button
                className="relative flex w-full cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none transition-colors text-red-600 hover:bg-accent hover:text-red-700"
                onClick={() => {
                  onDelete()
                  setContextMenuOpen(false)
                }}
              >
                Delete
              </button>
            )}
          </div>
        )}
      </span>
    )
  }

  const grouped = projects.reduce((acc, project) => {
    acc[project.id] = {
      projectName: project.name,
      items: conversations.filter(c => c.projectId === project.id),
    }
    return acc
  }, {} as Record<string, { projectName: string; items: typeof conversations }>)

  const unsortedConversations = conversations.filter(c => !c.projectId || c.projectId === "unsorted")

  return (
    <TooltipProvider delayDuration={0}>
      <div
        className={cn(
          "flex flex-col border-r border-border bg-sidebar transition-all duration-200 ease-linear h-full overflow-hidden",
          collapsed ? "w-16" : "w-64"
        )}
      >
        {/* Header */}
        <div className="flex h-16 items-center justify-between px-4 border-b border-border">
          {!collapsed && (
            <span className="font-display font-semibold text-lg text-sidebar-foreground">
              PrometheOS
            </span>
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                onClick={() => setCollapsed(!collapsed)}
                className="ml-auto"
              >
                {collapsed ? <PanelRight className="h-4 w-4" /> : <PanelLeft className="h-4 w-4" />}
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">
              {collapsed ? "Expand sidebar" : "Collapse sidebar"}
            </TooltipContent>
          </Tooltip>
        </div>

        {/* New Chat Button */}
        <div className="p-4">
          {collapsed ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => createConversation({ icon: selectedChatIcon })}
                  className="w-full"
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">New chat</TooltipContent>
            </Tooltip>
          ) : (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" className="w-full justify-start">
                  <Plus className="h-4 w-4 mr-2" />
                  New chat
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-48">
                {chatIconOptions.map((option) => (
                  <DropdownMenuItem
                    key={option.key}
                    onClick={() => {
                      setSelectedChatIcon(option.key)
                      createConversation({ icon: option.key })
                    }}
                  >
                    <option.Icon className="h-4 w-4 mr-2" />
                    {option.label}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>

        <Separator />

        {/* Content */}
        <ScrollArea className="flex-1">
          <div className="p-3 space-y-4">
            {/* Quick Actions */}
            {!collapsed && (
              <div className="space-y-1">
                <div className="text-xs font-semibold text-muted-foreground uppercase tracking-wider px-2">
                  Quick Actions
                </div>
                <div className="grid grid-cols-2 gap-1">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-8 text-xs justify-start"
                  >
                    <Plus className="h-3 w-3 mr-1.5" />
                    New Chat
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-8 text-xs justify-start"
                  >
                    <Folder className="h-3 w-3 mr-1.5" />
                    New Project
                  </Button>
                </div>
              </div>
            )}

            <Separator />

            {/* Projects */}
            {!collapsed && (
              <Collapsible defaultOpen className="space-y-2">
                <CollapsibleTrigger className="flex items-center justify-between w-full px-2 py-1.5 rounded-md hover:bg-sidebar-accent transition-colors">
                  <div className="flex items-center gap-2 text-sm font-medium text-sidebar-foreground">
                    <Folder className="h-4 w-4" />
                    <span>Projects</span>
                    <span className="text-xs text-muted-foreground ml-auto">
                      {projects.filter(p => p.id !== "unsorted").length}
                    </span>
                  </div>
                  <ChevronRight className="h-4 w-4 text-muted-foreground transition-transform" />
                </CollapsibleTrigger>
                <CollapsibleContent className="space-y-1 pl-2">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="sm" className="h-7 w-full justify-start text-xs">
                        <Plus className="h-3 w-3 mr-2" />
                        Add Project
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="start" className="w-48">
                      {projectIconOptions.map((option) => (
                        <DropdownMenuItem
                          key={option.key}
                          onClick={() => {
                            setSelectedProjectIcon(option.key)
                            createProject({ icon: option.key })
                          }}
                        >
                          <option.Icon className="h-4 w-4 mr-2" />
                          {option.label}
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuContent>
                  </DropdownMenu>
                  {projects.filter(p => p.id !== "unsorted").map((project) => (
                    <div key={project.id} className="space-y-1">
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-sidebar-accent cursor-pointer group">
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <div className="hover:bg-sidebar-accent/50 rounded p-1 group-hover:bg-sidebar-accent/30 transition-colors">
                              <RenderIcon name={project.icon} className="h-4 w-4" />
                            </div>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="start" className="w-48">
                            {projectIconOptions.map((option) => (
                              <DropdownMenuItem
                                key={option.key}
                                onClick={() => setProjectIcon(project.id, option.key)}
                              >
                                <option.Icon className="h-4 w-4 mr-2" />
                                {option.label}
                              </DropdownMenuItem>
                            ))}
                          </DropdownMenuContent>
                        </DropdownMenu>
                        <EditableText
                          value={project.name}
                          isEditing={editingProjectId === project.id}
                          onSave={(newName) => {
                            renameProject(project.id, newName)
                            setEditingProjectId(null)
                          }}
                          onCancel={() => setEditingProjectId(null)}
                          onStartEdit={() => setEditingProjectId(project.id)}
                          onEdit={() => setEditingProjectId(project.id)}
                          onDelete={() => deleteProject(project.id)}
                          onArchive={() => archiveProject(project.id)}
                          onDuplicate={() => duplicateProject(project.id)}
                          onAddChat={() => createConversation({ projectId: project.id })}
                          className="text-sm text-sidebar-foreground truncate flex-1"
                        />
                      </div>
                      {grouped[project.id]?.items.map((conv) => (
                        <div
                          key={conv.id}
                          className={cn(
                            "flex items-center gap-2 px-2 py-1.5 rounded-md cursor-pointer ml-4 text-sm transition-all duration-200 hover-glow",
                            currentConversationId === conv.id
                              ? "bg-sidebar-accent text-sidebar-accent-foreground"
                              : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                          )}
                          onClick={() => setCurrentConversation(conv.id)}
                        >
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild onClick={(e) => e.stopPropagation()}>
                              <div className="hover:bg-sidebar-accent/50 rounded p-1">
                                <RenderIcon name={conv.icon} className="h-4 w-4" />
                              </div>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="start" className="w-48">
                              {chatIconOptions.map((option) => (
                                <DropdownMenuItem
                                  key={option.key}
                                  onClick={() => setConversationIcon(conv.id, option.key)}
                                >
                                  <option.Icon className="h-4 w-4 mr-2" />
                                  {option.label}
                                </DropdownMenuItem>
                              ))}
                            </DropdownMenuContent>
                          </DropdownMenu>
                          <EditableText
                            value={conv.title}
                            isEditing={editingConversationId === conv.id}
                            onSave={(newTitle) => {
                              renameConversation(conv.id, newTitle)
                              setEditingConversationId(null)
                            }}
                            onCancel={() => setEditingConversationId(null)}
                            onStartEdit={(e) => {
                              e?.stopPropagation()
                              setEditingConversationId(conv.id)
                            }}
                            onEdit={() => setEditingConversationId(conv.id)}
                            onDelete={() => deleteConversation(conv.id)}
                            onArchive={() => archiveConversation(conv.id)}
                            onDuplicate={() => duplicateConversation(conv.id)}
                            className="truncate flex-1"
                          />
                        </div>
                      ))}
                    </div>
                  ))}
                </CollapsibleContent>
              </Collapsible>
            )}

            {/* Chats */}
            {!collapsed && (
              <Collapsible defaultOpen className="space-y-2">
                <CollapsibleTrigger className="flex items-center justify-between w-full px-2 py-1.5 rounded-md hover:bg-sidebar-accent transition-colors">
                  <div className="flex items-center gap-2 text-sm font-medium text-sidebar-foreground">
                    <MessageSquare className="h-4 w-4" />
                    <span>Chats</span>
                    <span className="text-xs text-muted-foreground ml-auto">
                      {unsortedConversations.length}
                    </span>
                  </div>
                  <ChevronRight className="h-4 w-4 text-muted-foreground transition-transform" />
                </CollapsibleTrigger>
                <CollapsibleContent className="space-y-1 pl-2">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="sm" className="h-7 w-full justify-start text-xs">
                        <Plus className="h-3 w-3 mr-2" />
                        Add Chat
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="start" className="w-48">
                      {chatIconOptions.map((option) => (
                        <DropdownMenuItem
                          key={option.key}
                          onClick={() => {
                            setSelectedChatIcon(option.key)
                            createConversation({ icon: option.key })
                          }}
                        >
                          <option.Icon className="h-4 w-4 mr-2" />
                          {option.label}
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuContent>
                  </DropdownMenu>
                  {unsortedConversations.map((conv) => (
                    <div
                      key={conv.id}
                      className={cn(
                        "flex items-center gap-2 px-2 py-1.5 rounded-md cursor-pointer text-sm transition-all duration-200 hover-glow",
                        currentConversationId === conv.id
                          ? "bg-sidebar-accent text-sidebar-accent-foreground"
                          : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                      )}
                      onClick={() => setCurrentConversation(conv.id)}
                    >
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild onClick={(e) => e.stopPropagation()}>
                          <div className="hover:bg-sidebar-accent/50 rounded p-1">
                            <RenderIcon name={conv.icon} className="h-4 w-4" />
                          </div>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="start" className="w-48">
                          {chatIconOptions.map((option) => (
                            <DropdownMenuItem
                              key={option.key}
                              onClick={() => setConversationIcon(conv.id, option.key)}
                            >
                              <option.Icon className="h-4 w-4 mr-2" />
                              {option.label}
                            </DropdownMenuItem>
                          ))}
                        </DropdownMenuContent>
                      </DropdownMenu>
                      <EditableText
                        value={conv.title}
                        isEditing={editingConversationId === conv.id}
                        onSave={(newTitle) => {
                          renameConversation(conv.id, newTitle)
                          setEditingConversationId(null)
                        }}
                        onCancel={() => setEditingConversationId(null)}
                        onStartEdit={(e) => {
                          e?.stopPropagation()
                          setEditingConversationId(conv.id)
                        }}
                        onEdit={() => setEditingConversationId(conv.id)}
                        onDelete={() => deleteConversation(conv.id)}
                        onArchive={() => archiveConversation(conv.id)}
                        onDuplicate={() => duplicateConversation(conv.id)}
                        className="truncate flex-1"
                      />
                    </div>
                  ))}
                </CollapsibleContent>
              </Collapsible>
            )}
          </div>
        </ScrollArea>

        {/* Footer */}
        <div className="p-4 border-t border-border">
          {collapsed ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button variant="ghost" size="icon" className="w-full">
                  <User className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">Profile</TooltipContent>
            </Tooltip>
          ) : (
            <Button variant="ghost" className="w-full justify-start">
              <User className="h-4 w-4 mr-2" />
              <span>Profile</span>
            </Button>
          )}
        </div>
      </div>
    </TooltipProvider>
  )
}
