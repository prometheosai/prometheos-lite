"use client"

import { useState, useEffect } from "react"
import { Command } from "cmdk"
import { Search, MessageSquare, Folder } from "lucide-react"
import { useChat } from "@/context/chat-context"
import { rankSearchResult, type SearchResult } from "@/lib/chat-utils"
import { cn } from "@/lib/utils"

interface SearchCommandPaletteProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function SearchCommandPalette({ open, onOpenChange }: SearchCommandPaletteProps) {
  const { projects, conversations, setCurrentConversation, setCurrentProject } = useChat()
  const [query, setQuery] = useState("")
  const [results, setResults] = useState<SearchResult[]>([])

  useEffect(() => {
    if (!query) {
      setResults([])
      return
    }

    const searchResults: SearchResult[] = []

    // Search projects
    for (const project of projects) {
      if (project.id === "unsorted") continue
      const score = rankSearchResult(query, { id: project.id, type: "project", title: project.name, score: 0 })
      if (score > 0) {
        searchResults.push({
          id: project.id,
          type: "project",
          title: project.name,
          score,
        })
      }
    }

    // Search chats
    for (const conv of conversations) {
      const chatScore = rankSearchResult(query, { id: conv.id, type: "chat", title: conv.title, score: 0 })
      if (chatScore > 0) {
        const project = projects.find(p => p.id === conv.projectId)
        searchResults.push({
          id: conv.id,
          type: "chat",
          title: conv.title,
          subtitle: project?.name || "Unassigned",
          projectId: conv.projectId || undefined,
          score: chatScore,
        })
      }

      // Search message content
      for (const msg of conv.messages) {
        const contentLower = msg.content.toLowerCase()
        if (contentLower.includes(query.toLowerCase())) {
          const project = projects.find(p => p.id === conv.projectId)
          searchResults.push({
            id: conv.id,
            type: "message",
            title: conv.title,
            subtitle: project?.name || "Unassigned",
            projectId: conv.projectId || undefined,
            chatId: conv.id,
            score: 20, // Lower score for message matches
          })
        }
      }
    }

    // Sort by score (descending)
    searchResults.sort((a, b) => b.score - a.score)

    setResults(searchResults)
  }, [query, projects, conversations])

  const handleSelect = (result: SearchResult) => {
    if (result.type === "project") {
      setCurrentProject(result.id)
    } else if (result.type === "chat" || result.type === "message") {
      setCurrentConversation(result.id)
      if (result.projectId) {
        setCurrentProject(result.projectId)
      }
    }
    onOpenChange(false)
    setQuery("")
  }

  const groupedResults = {
    projects: results.filter(r => r.type === "project"),
    chats: results.filter(r => r.type === "chat"),
    messages: results.filter(r => r.type === "message"),
  }

  return (
    <Command.Dialog open={open} onOpenChange={onOpenChange} shouldFilter={false}>
      <div className="fixed inset-0 bg-background/80 backdrop-blur-sm z-50" />
      <div className="fixed left-[50%] top-[20%] translate-x-[-50%] z-50 w-full max-w-lg">
        <Command className="rounded-lg border border-border bg-background shadow-md">
          <div className="flex items-center border-b border-border px-3">
            <Search className="h-4 w-4 text-muted-foreground mr-2" />
            <Command.Input
              value={query}
              onValueChange={setQuery}
              placeholder="Search chats and projects..."
              className="flex h-11 w-full rounded-md bg-transparent py-3 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50"
            />
          </div>
          <Command.List className="max-h-[300px] overflow-y-auto p-2">
            {query === "" && (
              <Command.Empty className="py-6 text-center text-sm text-muted-foreground">
                Start typing to search...
              </Command.Empty>
            )}
            {query !== "" && results.length === 0 && (
              <Command.Empty className="py-6 text-center text-sm text-muted-foreground">
                No results found.
              </Command.Empty>
            )}
            {groupedResults.projects.length > 0 && (
              <Command.Group heading="Projects">
                {groupedResults.projects.map((result) => (
                  <Command.Item
                    key={result.id}
                    onSelect={() => handleSelect(result)}
                    className="flex items-center gap-2 px-2 py-2 rounded-md text-sm cursor-pointer hover:bg-accent aria-selected:bg-accent aria-selected:text-accent-foreground"
                  >
                    <Folder className="h-4 w-4 text-muted-foreground" />
                    <span>{result.title}</span>
                  </Command.Item>
                ))}
              </Command.Group>
            )}
            {groupedResults.chats.length > 0 && (
              <Command.Group heading="Chats">
                {groupedResults.chats.map((result) => (
                  <Command.Item
                    key={result.id}
                    onSelect={() => handleSelect(result)}
                    className="flex flex-col px-2 py-2 rounded-md text-sm cursor-pointer hover:bg-accent aria-selected:bg-accent aria-selected:text-accent-foreground"
                  >
                    <div className="flex items-center gap-2">
                      <MessageSquare className="h-4 w-4 text-muted-foreground" />
                      <span className="font-medium">{result.title}</span>
                    </div>
                    {result.subtitle && (
                      <span className="text-xs text-muted-foreground ml-6">{result.subtitle}</span>
                    )}
                  </Command.Item>
                ))}
              </Command.Group>
            )}
            {groupedResults.messages.length > 0 && (
              <Command.Group heading="Messages">
                {groupedResults.messages.map((result) => (
                  <Command.Item
                    key={`${result.id}-msg`}
                    onSelect={() => handleSelect(result)}
                    className="flex flex-col px-2 py-2 rounded-md text-sm cursor-pointer hover:bg-accent aria-selected:bg-accent aria-selected:text-accent-foreground"
                  >
                    <div className="flex items-center gap-2">
                      <MessageSquare className="h-4 w-4 text-muted-foreground" />
                      <span className="font-medium">{result.title}</span>
                    </div>
                    {result.subtitle && (
                      <span className="text-xs text-muted-foreground ml-6">{result.subtitle} · message match</span>
                    )}
                  </Command.Item>
                ))}
              </Command.Group>
            )}
          </Command.List>
        </Command>
      </div>
    </Command.Dialog>
  )
}
