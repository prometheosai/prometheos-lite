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
  ChevronDown,
  Search,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { Separator } from "@/components/ui/separator"
import { useChat } from "@/context/chat-context"
import { cn } from "@/lib/utils"
import { groupChatsByTime, getChatPreview } from "@/lib/chat-utils"
import { SearchCommandPalette } from "./search-command-palette"
import { ProfileModal } from "./profile-modal"

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
  const [collapsedTimeGroups, setCollapsedTimeGroups] = useState<Record<string, Record<string, boolean>>>({})
  const [showMoreGroups, setShowMoreGroups] = useState<Record<string, Record<string, boolean>>>({})
  const [searchOpen, setSearchOpen] = useState(false)
  const [profileOpen, setProfileOpen] = useState(false)
  const { projects, conversations, currentConversationId, currentProjectId, createProject, createConversation, setCurrentConversation, setCurrentProject, setProjectIcon, setConversationIcon, renameProject, renameConversation, deleteProject, archiveProject, duplicateProject, deleteConversation, archiveConversation, duplicateConversation } = useChat()

  // Load collapsed time groups from localStorage on mount
  useEffect(() => {
    const savedCollapsedGroups = localStorage.getItem("sidebar:collapsedGroups")
    if (savedCollapsedGroups) {
      try {
        setCollapsedTimeGroups(JSON.parse(savedCollapsedGroups))
      } catch (e) {
        console.error("Failed to parse collapsed groups from localStorage:", e)
      }
    }
  }, [])

  // Save collapsed time groups to localStorage when they change
  useEffect(() => {
    localStorage.setItem("sidebar:collapsedGroups", JSON.stringify(collapsedTimeGroups))
  }, [collapsedTimeGroups])

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault()
        setSearchOpen(true)
      }
    }

    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [])

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
    const projectChats = conversations.filter(c => c.projectId === project.id)
    acc[project.id] = {
      projectName: project.name,
      items: projectChats,
      grouped: groupChatsByTime(projectChats),
    }
    return acc
  }, {} as Record<string, { projectName: string; items: typeof conversations; grouped: ReturnType<typeof groupChatsByTime> }>)

  const unsortedConversations = conversations.filter(c => !c.projectId || c.projectId === "unsorted")
  const unsortedGrouped = groupChatsByTime(unsortedConversations)

  const TimeGroupSection = ({
    project,
    grouped,
    currentConversationId,
    editingConversationId,
    setEditingConversationId,
    setCurrentConversation,
    setConversationIcon,
    renameConversation,
    deleteConversation,
    archiveConversation,
    duplicateConversation,
    collapsedTimeGroups,
    setCollapsedTimeGroups,
    showMoreGroups,
    setShowMoreGroups,
    RenderIcon,
    EditableText,
    chatIconOptions,
  }: {
    project: any
    grouped: ReturnType<typeof groupChatsByTime> | undefined
    currentConversationId: string | null
    editingConversationId: string | null
    setEditingConversationId: (id: string | null) => void
    setCurrentConversation: (id: string) => void
    setConversationIcon: (id: string, icon: string) => void
    renameConversation: (id: string, title: string) => void
    deleteConversation: (id: string) => void
    archiveConversation: (id: string) => void
    duplicateConversation: (id: string) => void
    collapsedTimeGroups: Record<string, Record<string, boolean>>
    setCollapsedTimeGroups: React.Dispatch<React.SetStateAction<Record<string, Record<string, boolean>>>>
    showMoreGroups: Record<string, Record<string, boolean>>
    setShowMoreGroups: React.Dispatch<React.SetStateAction<Record<string, Record<string, boolean>>>>
    RenderIcon: any
    EditableText: any
    chatIconOptions: readonly any[]
  }) => {
    if (!grouped) return null

    const timeGroups = [
      { key: "today", label: "Today", chats: grouped.today },
      { key: "thisWeek", label: "This Week", chats: grouped.thisWeek },
      { key: "older", label: "Older", chats: grouped.older },
    ]

    return (
      <>
        {timeGroups.map(({ key, label, chats }) => {
          if (chats.length === 0) return null

          const isCollapsed = collapsedTimeGroups[project.id]?.[key] ?? (key !== "today")
          const showMore = showMoreGroups[project.id]?.[key] ?? false
          const visibleChats = showMore ? chats : chats.slice(0, 7)

          return (
            <div key={key} className="ml-4">
              <Collapsible
                open={!isCollapsed}
                onOpenChange={(open) => {
                  setCollapsedTimeGroups(prev => ({
                    ...prev,
                    [project.id]: { ...prev[project.id], [key]: !open }
                  }))
                }}
              >
                <CollapsibleTrigger className="flex items-center gap-1 px-2 py-1 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors w-full">
                  <ChevronDown className={cn("h-3 w-3 transition-transform", isCollapsed && "-rotate-90")} />
                  <span>{label}</span>
                  <span className="text-muted-foreground">({chats.length})</span>
                </CollapsibleTrigger>
                <CollapsibleContent className="space-y-1">
                  {visibleChats.map((conv) => (
                    <Tooltip key={conv.id}>
                      <TooltipTrigger asChild>
                        <div
                          className={cn(
                            "flex items-center gap-2 px-2 py-1.5 rounded-md cursor-pointer ml-2 text-sm transition-all duration-200",
                            currentConversationId === conv.id
                              ? "bg-sidebar-accent/60 text-sidebar-accent-foreground"
                              : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                          )}
                          onClick={() => setCurrentConversation(conv.id)}
                          aria-current={currentConversationId === conv.id ? "page" : undefined}
                        >
                          {currentConversationId === conv.id && (
                            <div className="h-1.5 w-1.5 rounded-full bg-primary" />
                          )}
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild onClick={(e: React.MouseEvent) => e.stopPropagation()}>
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
                            onSave={(newTitle: string) => {
                              renameConversation(conv.id, newTitle)
                              setEditingConversationId(null)
                            }}
                            onCancel={() => setEditingConversationId(null)}
                            onStartEdit={(e?: React.MouseEvent) => {
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
                      </TooltipTrigger>
                      <TooltipContent side="right" className="max-w-xs">
                        <p className="text-xs">{getChatPreview(conv)}</p>
                      </TooltipContent>
                    </Tooltip>
                  ))}
                  {chats.length > 7 && !showMore && (
                    <button
                      className="ml-2 px-2 py-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
                      onClick={() => setShowMoreGroups(prev => ({
                        ...prev,
                        [project.id]: { ...prev[project.id], [key]: true }
                      }))}
                    >
                      Show more ({chats.length - 7} remaining)
                    </button>
                  )}
                </CollapsibleContent>
              </Collapsible>
            </div>
          )
        })}
      </>
    )
  }

  const UnassignedTimeGroupSection = ({
    grouped,
    currentConversationId,
    editingConversationId,
    setEditingConversationId,
    setCurrentConversation,
    setConversationIcon,
    renameConversation,
    deleteConversation,
    archiveConversation,
    duplicateConversation,
    collapsedTimeGroups,
    setCollapsedTimeGroups,
    showMoreGroups,
    setShowMoreGroups,
    RenderIcon,
    EditableText,
    chatIconOptions,
  }: {
    grouped: ReturnType<typeof groupChatsByTime>
    currentConversationId: string | null
    editingConversationId: string | null
    setEditingConversationId: (id: string | null) => void
    setCurrentConversation: (id: string) => void
    setConversationIcon: (id: string, icon: string) => void
    renameConversation: (id: string, title: string) => void
    deleteConversation: (id: string) => void
    archiveConversation: (id: string) => void
    duplicateConversation: (id: string) => void
    collapsedTimeGroups: Record<string, Record<string, boolean>>
    setCollapsedTimeGroups: React.Dispatch<React.SetStateAction<Record<string, Record<string, boolean>>>>
    showMoreGroups: Record<string, Record<string, boolean>>
    setShowMoreGroups: React.Dispatch<React.SetStateAction<Record<string, Record<string, boolean>>>>
    RenderIcon: any
    EditableText: any
    chatIconOptions: readonly any[]
  }) => {
    const timeGroups = [
      { key: "today", label: "Today", chats: grouped.today },
      { key: "thisWeek", label: "This Week", chats: grouped.thisWeek },
      { key: "older", label: "Older", chats: grouped.older },
    ]

    return (
      <>
        {timeGroups.map(({ key, label, chats }) => {
          if (chats.length === 0) return null

          const isCollapsed = collapsedTimeGroups["unassigned"]?.[key] ?? (key !== "today")
          const showMore = showMoreGroups["unassigned"]?.[key] ?? false
          const visibleChats = showMore ? chats : chats.slice(0, 7)

          return (
            <div key={key}>
              <Collapsible
                open={!isCollapsed}
                onOpenChange={(open) => {
                  setCollapsedTimeGroups(prev => ({
                    ...prev,
                    unassigned: { ...prev.unassigned, [key]: !open }
                  }))
                }}
              >
                <CollapsibleTrigger className="flex items-center gap-1 px-2 py-1 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors w-full">
                  <ChevronDown className={cn("h-3 w-3 transition-transform", isCollapsed && "-rotate-90")} />
                  <span>{label}</span>
                  <span className="text-muted-foreground">({chats.length})</span>
                </CollapsibleTrigger>
                <CollapsibleContent className="space-y-1">
                  {visibleChats.map((conv) => (
                    <Tooltip key={conv.id}>
                      <TooltipTrigger asChild>
                        <div
                          className={cn(
                            "flex items-center gap-2 px-2 py-1.5 rounded-md cursor-pointer text-sm transition-all duration-200",
                            currentConversationId === conv.id
                              ? "bg-sidebar-accent/60 text-sidebar-accent-foreground"
                              : "text-sidebar-foreground hover:bg-sidebar-accent/50"
                          )}
                          onClick={() => setCurrentConversation(conv.id)}
                          aria-current={currentConversationId === conv.id ? "page" : undefined}
                        >
                          {currentConversationId === conv.id && (
                            <div className="h-1.5 w-1.5 rounded-full bg-primary" />
                          )}
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild onClick={(e: React.MouseEvent) => e.stopPropagation()}>
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
                            onSave={(newTitle: string) => {
                              renameConversation(conv.id, newTitle)
                              setEditingConversationId(null)
                            }}
                            onCancel={() => setEditingConversationId(null)}
                            onStartEdit={(e?: React.MouseEvent) => {
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
                      </TooltipTrigger>
                      <TooltipContent side="right" className="max-w-xs">
                        <p className="text-xs">{getChatPreview(conv)}</p>
                      </TooltipContent>
                    </Tooltip>
                  ))}
                  {chats.length > 7 && !showMore && (
                    <button
                      className="px-2 py-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
                      onClick={() => setShowMoreGroups(prev => ({
                        ...prev,
                        unassigned: { ...prev.unassigned, [key]: true }
                      }))}
                    >
                      Show more ({chats.length - 7} remaining)
                    </button>
                  )}
                </CollapsibleContent>
              </Collapsible>
            </div>
          )
        })}
      </>
    )
  }

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
                aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
              >
                {collapsed ? <PanelRight className="h-4 w-4" /> : <PanelLeft className="h-4 w-4" />}
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">
              {collapsed ? "Expand sidebar" : "Collapse sidebar"}
            </TooltipContent>
          </Tooltip>
        </div>

        {/* Search */}
        <div className="px-4 pb-4">
          {collapsed ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => setSearchOpen(true)}
                  className="w-full"
                  aria-label="Search chats and projects"
                >
                  <Search className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">Search (⌘K)</TooltipContent>
            </Tooltip>
          ) : (
            <Button
              variant="outline"
              className="w-full justify-start text-muted-foreground"
              onClick={() => setSearchOpen(true)}
              aria-label="Search chats and projects"
            >
              <Search className="h-4 w-4 mr-2" />
              Search...
              <span className="ml-auto text-xs text-muted-foreground">⌘K</span>
            </Button>
          )}
        </div>

        {/* New Chat Button */}
        <div className="px-4 pb-4">
          {collapsed ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => createConversation({ icon: selectedChatIcon })}
                  className="w-full"
                  aria-label="Create new chat"
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">New chat</TooltipContent>
            </Tooltip>
          ) : (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" className="w-full justify-start" aria-label="Create new chat">
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
                  {projects.filter(p => p.id !== "unsorted").length === 0 ? (
                    <div className="px-2 py-4 text-center">
                      <p className="text-xs text-muted-foreground">No projects yet</p>
                      <p className="text-xs text-muted-foreground">Create a project to organize your chats</p>
                    </div>
                  ) : (
                    projects.filter(p => p.id !== "unsorted").map((project) => (
                      <div key={project.id} className="space-y-1">
                        <div
                          className={cn(
                            "flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-sidebar-accent cursor-pointer group transition-all",
                            currentProjectId === project.id
                              ? "bg-sidebar-accent/80 border-l-2 border-primary"
                              : ""
                          )}
                          onClick={() => setCurrentProject(project.id)}
                        >
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild onClick={(e) => e.stopPropagation()}>
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
                            className={cn(
                              "text-sm text-sidebar-foreground truncate flex-1",
                              currentProjectId === project.id ? "font-semibold" : ""
                            )}
                          />
                        </div>
                        <TimeGroupSection
                          project={project}
                          grouped={grouped[project.id]?.grouped}
                          currentConversationId={currentConversationId}
                          editingConversationId={editingConversationId}
                          setEditingConversationId={setEditingConversationId}
                          setCurrentConversation={setCurrentConversation}
                          setConversationIcon={setConversationIcon}
                          renameConversation={renameConversation}
                          deleteConversation={deleteConversation}
                          archiveConversation={archiveConversation}
                          duplicateConversation={duplicateConversation}
                          collapsedTimeGroups={collapsedTimeGroups}
                          setCollapsedTimeGroups={setCollapsedTimeGroups}
                          showMoreGroups={showMoreGroups}
                          setShowMoreGroups={setShowMoreGroups}
                          RenderIcon={RenderIcon}
                          EditableText={EditableText}
                          chatIconOptions={chatIconOptions}
                        />
                      </div>
                    ))
                  )}
                </CollapsibleContent>
              </Collapsible>
            )}

            {/* Unassigned Chats */}
            {!collapsed && (
              <Collapsible defaultOpen className="space-y-2">
                <CollapsibleTrigger className="flex items-center justify-between w-full px-2 py-1.5 rounded-md hover:bg-sidebar-accent transition-colors">
                  <div className="flex items-center gap-2 text-sm font-medium text-sidebar-foreground">
                    <MessageSquare className="h-4 w-4" />
                    <span>Unassigned Chats</span>
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
                  {unsortedConversations.length === 0 ? (
                    <div className="px-2 py-4 text-center">
                      <p className="text-xs text-muted-foreground">No unassigned chats</p>
                      <p className="text-xs text-muted-foreground">Create a chat or add it to a project</p>
                    </div>
                  ) : (
                    <UnassignedTimeGroupSection
                      grouped={unsortedGrouped}
                      currentConversationId={currentConversationId}
                      editingConversationId={editingConversationId}
                      setEditingConversationId={setEditingConversationId}
                      setCurrentConversation={setCurrentConversation}
                      setConversationIcon={setConversationIcon}
                      renameConversation={renameConversation}
                      deleteConversation={deleteConversation}
                      archiveConversation={archiveConversation}
                      duplicateConversation={duplicateConversation}
                      collapsedTimeGroups={collapsedTimeGroups}
                      setCollapsedTimeGroups={setCollapsedTimeGroups}
                      showMoreGroups={showMoreGroups}
                      setShowMoreGroups={setShowMoreGroups}
                      RenderIcon={RenderIcon}
                      EditableText={EditableText}
                      chatIconOptions={chatIconOptions}
                    />
                  )}
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
                <Button
                  variant="ghost"
                  size="icon"
                  className="w-full"
                  onClick={() => setProfileOpen(true)}
                  aria-label="Open profile settings"
                >
                  <User className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">Profile</TooltipContent>
            </Tooltip>
          ) : (
            <Button
              variant="ghost"
              className="w-full justify-start"
              onClick={() => setProfileOpen(true)}
              aria-label="Open profile settings"
            >
              <User className="h-4 w-4 mr-2" />
              <span>Profile</span>
            </Button>
          )}
        </div>

        <SearchCommandPalette open={searchOpen} onOpenChange={setSearchOpen} />
        <ProfileModal open={profileOpen} onOpenChange={setProfileOpen} />
      </div>
    </TooltipProvider>
  )
}
