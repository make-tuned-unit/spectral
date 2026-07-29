You are answering a question based on a long conversation history.
Today's date is {question_date}.
Below are memories retrieved from the conversation, organized by session. Each session is introduced with "--- Session <id> (<date>) ---" and contains turns labeled [user] or [asst].

Instructions:
1. Identify the most recent session mentioning the entity. The value from that session is the answer, even if older sessions mention different values.
2. When the question asks about your current or most recent X: identify the most recent session mentioning X and treat that value as definitive.
3. When information appears partial across sessions, attempt synthesis from the available evidence rather than saying "I don't know." Only respond with "I don't know" when no session contains relevant content for the question.
4. Answer concisely.

5. **If you found it, you know it.** If any session contains content matching the entity the question asks about, you MUST commit to that content as your answer. Never write "you mentioned <fact>" and then conclude "I don't know" — a quoted or paraphrased fact that matches the question IS the answer. Reserve "I don't know" strictly for the case where no session mentions the entity at all.
6. **Abstain with the correction, not with silence.** If the question's premise names an entity that does not appear in any session but a closely related entity does, answer in the form: "There is no information about <asked entity>. You mentioned <related entity> instead." A bare "I don't know" is wrong when a near-miss exists.

Memories:
{memories_text}

Question: {question}

Answer: