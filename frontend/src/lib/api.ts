const API_BASE = 'http://127.0.0.1:3000';

export interface Project {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
}

export interface Conversation {
  id: string;
  project_id: string;
  title: string;
  created_at: string;
  updated_at: string;
}

export interface Message {
  id: string;
  conversation_id: string;
  role: string;
  content: string;
  created_at: string;
}

export interface FlowRun {
  id: string;
  conversation_id: string;
  status: string;
  started_at: string;
  completed_at: string | null;
}

export interface FlowEvent {
  type: 'node_start' | 'node_end' | 'output' | 'error';
  data: {
    node?: string;
    timestamp: string;
    data?: string;
    message?: string;
  };
}

export interface RuntimeModelStack {
  provider: string;
  provider_label: string;
  primary_model: string;
  fallback_models: string[];
  embedding_model: string;
  embedding_dimension: number;
}

// Projects API
export async function getProjects(): Promise<Project[]> {
  const res = await fetch(`${API_BASE}/projects`);
  if (!res.ok) throw new Error('Failed to fetch projects');
  return res.json();
}

export async function createProject(name: string): Promise<Project> {
  const res = await fetch(`${API_BASE}/projects`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name }),
  });
  if (!res.ok) throw new Error('Failed to create project');
  return res.json();
}

// Conversations API
export async function getConversations(projectId: string): Promise<Conversation[]> {
  const res = await fetch(`${API_BASE}/projects/${projectId}/conversations`);
  if (!res.ok) throw new Error('Failed to fetch conversations');
  return res.json();
}

export async function createConversation(projectId: string, title: string): Promise<Conversation> {
  const res = await fetch(`${API_BASE}/conversations`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ project_id: projectId, title }),
  });
  if (!res.ok) throw new Error('Failed to create conversation');
  return res.json();
}

// Messages API
export async function getMessages(conversationId: string): Promise<Message[]> {
  const res = await fetch(`${API_BASE}/conversations/${conversationId}/messages`);
  if (!res.ok) throw new Error('Failed to fetch messages');
  return res.json();
}

export async function createMessage(conversationId: string, role: string, content: string): Promise<Message> {
  const res = await fetch(`${API_BASE}/messages`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ conversation_id: conversationId, role, content }),
  });
  if (!res.ok) throw new Error('Failed to create message');
  return res.json();
}

// Flow Execution API
export async function runFlow(conversationId: string, message: string): Promise<FlowRun> {
  const res = await fetch(`${API_BASE}/conversations/${conversationId}/run`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message }),
  });
  if (!res.ok) throw new Error('Failed to run flow');
  return res.json();
}

// WebSocket connection
export function connectWebSocket(runId: string, onEvent: (event: FlowEvent) => void): WebSocket {
  const ws = new WebSocket(`ws://127.0.0.1:3000/ws/runs/${runId}`);

  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data) as FlowEvent;
      onEvent(data);
    } catch (e) {
      console.error('Failed to parse WebSocket message:', e);
    }
  };

  return ws;
}

export async function getRuntimeModelStack(): Promise<RuntimeModelStack> {
  const res = await fetch(`${API_BASE}/runtime/stack`);
  if (!res.ok) throw new Error('Failed to fetch runtime stack');
  return res.json();
}
