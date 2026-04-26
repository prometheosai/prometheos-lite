import { serve } from "https://deno.land/std@0.168.0/http/server.ts"

const corsHeaders = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Headers': 'authorization, x-client-info, apikey, content-type',
}

serve(async (req) => {
  if (req.method === 'OPTIONS') {
    return new Response('ok', { headers: corsHeaders })
  }

  try {
    const { message, mode, history } = await req.json()

    const openaiApiKey = Deno.env.get('OPENAI_API_KEY') || Deno.env.get('OPEN_AI_KEY')
    if (!openaiApiKey) {
      throw new Error('OpenAI API key not found')
    }

    // Customize system prompt based on mode
    let systemPrompt = "You are PrometheOS, a helpful AI assistant."
    
    switch (mode) {
      case 'create':
        systemPrompt = "You are PrometheOS in creation mode. Help users build, design, and create things. Be innovative and practical."
        break
      case 'research':
        systemPrompt = "You are PrometheOS in research mode. Provide thorough, accurate information and help users investigate topics deeply."
        break
      case 'tools':
        systemPrompt = "You are PrometheOS in tools mode. Help users with practical tools, utilities, and problem-solving approaches."
        break
      default:
        systemPrompt = "You are PrometheOS, a helpful AI assistant. Adapt your responses to what the user needs."
    }

    const response = await fetch('https://api.openai.com/v1/chat/completions', {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${openaiApiKey}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        model: 'gpt-4.1-2025-04-14',
        messages: [
          { role: 'system', content: systemPrompt },
          ...(Array.isArray(history) ? history.map((m: any) => ({ role: m.role, content: m.content })) : []),
          { role: 'user', content: message }
        ],
        temperature: 0.7,
        max_tokens: 1000,
      }),
    })

    const data = await response.json()

    if (!response.ok) {
      throw new Error(data.error?.message || 'OpenAI API error')
    }

    const aiResponse = data.choices[0]?.message?.content || 'No response generated'

    return new Response(
      JSON.stringify({ response: aiResponse }),
      {
        headers: { ...corsHeaders, 'Content-Type': 'application/json' },
        status: 200,
      },
    )
  } catch (error) {
    return new Response(
      JSON.stringify({ error: error.message }),
      {
        headers: { ...corsHeaders, 'Content-Type': 'application/json' },
        status: 400,
      },
    )
  }
})