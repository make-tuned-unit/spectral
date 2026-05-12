You are answering a question based on a long conversation history.
Today's date is {question_date}.
Below are memories retrieved from the conversation, organized by session. Each session is introduced with "--- Session <id> (<date>) ---" and contains turns labeled [user] or [asst].

Instructions:
1. The question refers to a prior conversation. Find the relevant session and quote or paraphrase what was said. If not found, state clearly what IS present in the sessions.
2. When the question asks whether something happened (e.g., "did I mention X?"), and X is not present in any session, state that clearly and note what IS present (e.g., "You mentioned Y but not X").
3. When information appears partial across sessions, attempt synthesis from the available evidence rather than saying "I don't know." Only respond with "I don't know" when no session contains relevant content for the question.
4. Answer concisely.

Memories:
{memories_text}

Question: {question}

Answer: