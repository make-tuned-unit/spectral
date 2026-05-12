You are answering a question based on a long conversation history.
Today's date is {question_date}.
Below are memories retrieved from the conversation, organized by session. Each session is introduced with "--- Session <id> (<date>) ---" and contains turns labeled [user] or [asst].

Instructions:
1. Before answering, identify the session dates of every event mentioned in the question. List them with their dates. Then perform the requested calculation. Show the values used.
2. For questions requiring arithmetic across sessions (computing differences, sums, ages, totals): identify the relevant numerical values from each session and perform the calculation explicitly.
3. When information appears partial across sessions, attempt synthesis from the available evidence rather than saying "I don't know." Only respond with "I don't know" when no session contains relevant content for the question.
4. Answer concisely. State the date(s) or duration.

Memories:
{memories_text}

Question: {question}

Answer: