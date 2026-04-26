import { format, isToday, isThisWeek, subDays, parseISO } from "date-fns"
import type { ChatConversation, ChatProject } from "@/context/chat-context"

export interface TimeGroupedChats {
  today: ChatConversation[]
  thisWeek: ChatConversation[]
  older: ChatConversation[]
}

export function groupChatsByTime(chats: ChatConversation[]): TimeGroupedChats {
  const now = new Date()
  const oneWeekAgo = subDays(now, 7)

  const grouped: TimeGroupedChats = {
    today: [],
    thisWeek: [],
    older: [],
  }

  for (const chat of chats) {
    const chatDate = new Date(chat.createdAt)
    
    if (isToday(chatDate)) {
      grouped.today.push(chat)
    } else if (isThisWeek(chatDate, { weekStartsOn: 1 })) {
      grouped.thisWeek.push(chat)
    } else {
      grouped.older.push(chat)
    }
  }

  return grouped
}

export function getChatPreview(chat: ChatConversation): string {
  if (!chat.messages || chat.messages.length === 0) {
    return "No messages yet"
  }
  
  const firstMessage = chat.messages[0]
  const preview = firstMessage.content.split("\n")[0].slice(0, 100)
  return preview.length === 100 ? preview + "..." : preview
}

export function getProjectChats(projectId: string, chats: ChatConversation[]): ChatConversation[] {
  return chats.filter(chat => chat.projectId === projectId)
}

export function getUnassignedChats(projects: ChatProject[], chats: ChatConversation[]): ChatConversation[] {
  const projectIds = projects.map(p => p.id)
  return chats.filter(chat => !chat.projectId || !projectIds.includes(chat.projectId))
}

export interface SearchResult {
  id: string
  type: "project" | "chat" | "message"
  title: string
  subtitle?: string
  projectId?: string
  chatId?: string
  score: number
}

export function rankSearchResult(query: string, item: SearchResult): number {
  const queryLower = query.toLowerCase()
  const titleLower = item.title.toLowerCase()
  const subtitleLower = item.subtitle?.toLowerCase() || ""

  // Exact title match
  if (titleLower === queryLower) {
    return 100
  }

  // Title starts with query
  if (titleLower.startsWith(queryLower)) {
    return 80
  }

  // Title includes query
  if (titleLower.includes(queryLower)) {
    return 60
  }

  // Subtitle includes query
  if (subtitleLower.includes(queryLower)) {
    return 40
  }

  // Message content match (already scored lower by type)
  if (item.type === "message") {
    return 20
  }

  return 0
}
