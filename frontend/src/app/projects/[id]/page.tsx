'use client';

import { useState, useEffect } from 'react';
import { Plus, MessageSquare, ArrowLeft } from 'lucide-react';
import { getConversations, createConversation, type Conversation } from '@/lib/api';
import { useRouter } from 'next/navigation';

export default function ProjectPage({ params }: { params: { id: string } }) {
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [newConversationTitle, setNewConversationTitle] = useState('');
  const [loading, setLoading] = useState(true);
  const router = useRouter();

  useEffect(() => {
    loadConversations();
  }, [params.id]);

  const loadConversations = async () => {
    try {
      const data = await getConversations(params.id);
      setConversations(data);
    } catch (error) {
      console.error('Failed to load conversations:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleCreateConversation = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newConversationTitle.trim()) return;

    try {
      const conversation = await createConversation(params.id, newConversationTitle);
      setConversations([...conversations, conversation]);
      setNewConversationTitle('');
    } catch (error) {
      console.error('Failed to create conversation:', error);
    }
  };

  const handleSelectConversation = (conversationId: string) => {
    router.push(`/conversations/${conversationId}`);
  };

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-600">Loading conversations...</div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 p-8">
      <div className="max-w-4xl mx-auto">
        <button
          onClick={() => router.push('/')}
          className="mb-6 flex items-center gap-2 text-gray-600 hover:text-gray-900"
        >
          <ArrowLeft size={20} />
          Back to Projects
        </button>

        <h1 className="text-3xl font-bold text-gray-900 mb-8">Conversations</h1>

        {/* Create Conversation Form */}
        <form onSubmit={handleCreateConversation} className="mb-8 flex gap-4">
          <input
            type="text"
            value={newConversationTitle}
            onChange={(e) => setNewConversationTitle(e.target.value)}
            placeholder="New conversation title"
            className="flex-1 px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
          <button
            type="submit"
            className="px-6 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 flex items-center gap-2"
          >
            <Plus size={20} />
            New Conversation
          </button>
        </form>

        {/* Conversations List */}
        <div className="grid gap-4">
          {conversations.length === 0 ? (
            <div className="text-center text-gray-500 py-12">
              No conversations yet. Create your first conversation to get started.
            </div>
          ) : (
            conversations.map((conversation) => (
              <div
                key={conversation.id}
                onClick={() => handleSelectConversation(conversation.id)}
                className="bg-white p-6 rounded-lg shadow-sm hover:shadow-md cursor-pointer transition-shadow"
              >
                <div className="flex items-center gap-3">
                  <MessageSquare size={24} className="text-blue-600" />
                  <div>
                    <h2 className="text-xl font-semibold text-gray-900">{conversation.title}</h2>
                    <p className="text-sm text-gray-500 mt-1">
                      Created: {new Date(conversation.created_at).toLocaleDateString()}
                    </p>
                  </div>
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
