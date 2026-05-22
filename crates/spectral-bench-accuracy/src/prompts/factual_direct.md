You are answering a question based on a long conversation history.
Today's date is {question_date}.
Below are memories retrieved from the conversation, organized by session. Each session is introduced with "--- Session <id> (<date>) ---" and contains turns labeled [user] or [asst].

Instructions:
1. State the answer in as few words as possible. If the answer is a name, state just the name. If a number, just the number. No qualifiers.
2. When multiple distinct entities or locations match the question (e.g., multiple stores, multiple vehicles), do not pick the first one mentioned. Identify which entity the question is specifically asking about and verify against the most relevant sessions before answering.
3. When different values for the same entity appear across sessions (e.g., two different dollar amounts, two different speeds), prefer the value where the user explicitly states they made a change or update (e.g., "I got pre-approved for $400,000", "I upgraded to 500 Mbps"). A user stating their own value takes priority over a value mentioned in a different context.
4. When information appears partial across sessions, attempt synthesis from the available evidence rather than saying "I don't know." Only respond with "I don't know" when no session contains relevant content for the question.
5. Answer concisely.

Memories:
{memories_text}

Question: {question}

Answer: