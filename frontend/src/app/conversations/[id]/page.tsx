'use client';

import { useState, useEffect, useRef } from 'react';
import { Send, ArrowLeft, Clock, FileText, Loader2, Moon, Sun } from 'lucide-react';
import { getMessages, runFlow, connectWebSocket, type Message, type FlowEvent } from '@/lib/api';
import { useRouter } from 'next/navigation';

export default function ConversationPage({ params }: { params: { id: string } }) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [events, setEvents] = useState<FlowEvent[]>([]);
  const [darkMode, setDarkMode] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const router = useRouter();

  useEffect(() => {
    loadMessages();
    return () => {
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, [params.id]);

  useEffect(() => {
    scrollToBottom();
  }, [messages, events]);

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  const loadMessages = async () => {
    try {
      const data = await getMessages(params.id);
      setMessages(data);
    } catch (error) {
      console.error('Failed to load messages:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleSendMessage = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!input.trim() || running) return;

    const userMessage = input;
    setInput('');
    setRunning(true);
    setEvents([]);

    try {
      // Run flow which saves the message and starts execution
      const flowRun = await runFlow(params.id, userMessage);
      
      // Connect WebSocket for live updates
      const ws = connectWebSocket(flowRun.id, (event) => {
        setEvents((prev) => [...prev, event]);
        
        // Check if flow is complete (after memory_write node)
        if (event.type === 'node_end' && event.data.node === 'memory_write') {
          setRunning(false);
          // Reload messages to get assistant response
          setTimeout(loadMessages, 500);
        }
        
        // Handle errors
        if (event.type === 'error') {
          setRunning(false);
        }
      });
      
      wsRef.current = ws;
    } catch (error) {
      console.error('Failed to run flow:', error);
      setRunning(false);
    }
  };

  const getTimelineStatus = () => {
    if (events.length === 0) return 'Idle';
    const lastEvent = events[events.length - 1];
    if (lastEvent.type === 'error') return 'Error occurred';
    if (lastEvent.type === 'node_start') return `Running: ${lastEvent.data.node}`;
    if (lastEvent.type === 'node_end') return 'Completed';
    return 'Processing';
  };

  return (
    <div className={`min-h-screen flex ${darkMode ? 'bg-gray-900' : 'bg-gray-50'}`}>
      {/* Main Chat Area */}
      <div className="flex-1 flex flex-col">
        {/* Header */}
        <div className={`${darkMode ? 'bg-gray-800 border-gray-700' : 'bg-white border-b'} px-6 py-4 flex items-center gap-4`}>
          <button
            onClick={() => router.back()}
            className={darkMode ? 'text-gray-400 hover:text-gray-200' : 'text-gray-600 hover:text-gray-900'}
          >
            <ArrowLeft size={24} />
          </button>
          <h1 className={`text-xl font-semibold ${darkMode ? 'text-gray-100' : 'text-gray-900'}`}>Chat</h1>
          <button
            onClick={() => setDarkMode(!darkMode)}
            className={`ml-auto ${darkMode ? 'text-gray-400 hover:text-gray-200' : 'text-gray-600 hover:text-gray-900'}`}
          >
            {darkMode ? <Sun size={20} /> : <Moon size={20} />}
          </button>
        </div>

        {/* Messages */}
        <div className={`flex-1 overflow-y-auto p-6 ${darkMode ? 'bg-gray-900' : ''}`}>
          {loading ? (
            <div className="flex items-center justify-center h-full">
              <Loader2 className={`animate-spin ${darkMode ? 'text-gray-600' : 'text-gray-400'}`} size={32} />
            </div>
          ) : messages.length === 0 ? (
            <div className={`text-center py-12 ${darkMode ? 'text-gray-500' : 'text-gray-500'}`}>
              No messages yet. Start a conversation!
            </div>
          ) : (
            <div className="space-y-4">
              {messages.map((message) => (
                <div
                  key={message.id}
                  className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}
                >
                  <div
                    className={`max-w-2xl px-4 py-2 rounded-lg ${
                      message.role === 'user'
                        ? 'bg-blue-600 text-white'
                        : darkMode
                        ? 'bg-gray-800 border border-gray-700 text-gray-100'
                        : 'bg-white border shadow-sm'
                    }`}
                  >
                    <p className="whitespace-pre-wrap">{message.content}</p>
                    <p className="text-xs mt-1 opacity-70">
                      {new Date(message.created_at).toLocaleTimeString()}
                    </p>
                  </div>
                </div>
              ))}
              <div ref={messagesEndRef} />
            </div>
          )}
        </div>

        {/* Input */}
        <div className={`${darkMode ? 'bg-gray-800 border-gray-700' : 'bg-white border-t'} p-4`}>
          <form onSubmit={handleSendMessage} className="flex gap-4">
            <input
              type="text"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder="Type your message..."
              disabled={running}
              className={`flex-1 px-4 py-2 border rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50 ${
                darkMode
                  ? 'bg-gray-700 border-gray-600 text-gray-100 placeholder-gray-400'
                  : 'border-gray-300'
              }`}
            />
            <button
              type="submit"
              disabled={running || !input.trim()}
              className="px-6 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
            >
              {running ? <Loader2 className="animate-spin" size={20} /> : <Send size={20} />}
              {running ? 'Running...' : 'Send'}
            </button>
          </form>
        </div>
      </div>

      {/* Sidebar - Timeline & Events */}
      <div className={`w-80 border-l flex flex-col ${darkMode ? 'bg-gray-800 border-gray-700' : 'bg-white'}`}>
        {/* Timeline */}
        <div className={`border-b p-4 ${darkMode ? 'border-gray-700' : ''}`}>
          <h2 className={`text-lg font-semibold flex items-center gap-2 ${darkMode ? 'text-gray-100' : 'text-gray-900'}`}>
            <Clock size={20} />
            Execution Timeline
          </h2>
          <p className={`text-sm mt-1 ${darkMode ? 'text-gray-400' : 'text-gray-500'}`}>{getTimelineStatus()}</p>
        </div>

        {/* Events */}
        <div className="flex-1 overflow-y-auto p-4">
          {events.length === 0 ? (
            <div className={`text-center py-8 text-sm ${darkMode ? 'text-gray-500' : 'text-gray-500'}`}>
              No events yet. Send a message to start flow execution.
            </div>
          ) : (
            <div className="space-y-2">
              {events.map((event, index) => (
                <div
                  key={index}
                  className={`p-3 rounded-lg text-sm ${
                    event.type === 'error'
                      ? darkMode
                        ? 'bg-red-900/30 border border-red-800 text-red-200'
                        : 'bg-red-50 border border-red-200'
                      : event.type === 'output'
                      ? darkMode
                        ? 'bg-green-900/30 border border-green-800 text-green-200'
                        : 'bg-green-50 border border-green-200'
                      : darkMode
                      ? 'bg-gray-700 border border-gray-600 text-gray-200'
                      : 'bg-gray-50 border border-gray-200'
                  }`}
                >
                  <div className="font-medium">
                    {event.type === 'node_start' && `Started: ${event.data.node}`}
                    {event.type === 'node_end' && `Completed: ${event.data.node}`}
                    {event.type === 'output' && `Output: ${event.data.node}`}
                    {event.type === 'error' && `Error: ${event.data.node}`}
                  </div>
                  {event.data.data && (
                    <div className={`mt-1 ${darkMode ? 'text-gray-300' : 'text-gray-600'}`}>{event.data.data}</div>
                  )}
                  {event.data.message && (
                    <div className="mt-1 text-red-600">{event.data.message}</div>
                  )}
                  <div className={`text-xs mt-1 ${darkMode ? 'text-gray-500' : 'text-gray-400'}`}>
                    {new Date(event.data.timestamp).toLocaleTimeString()}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Artifacts Placeholder */}
        <div className={`border-t p-4 ${darkMode ? 'border-gray-700' : ''}`}>
          <h2 className={`text-lg font-semibold flex items-center gap-2 ${darkMode ? 'text-gray-100' : 'text-gray-900'}`}>
            <FileText size={20} />
            Artifacts
          </h2>
          <p className={`text-sm mt-1 ${darkMode ? 'text-gray-400' : 'text-gray-500'}`}>
            Generated files will appear here
          </p>
        </div>
      </div>
    </div>
  );
}
