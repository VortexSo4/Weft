# AI policy

Read this before you open a pull request. The template asks you to confirm that you did.

Using AI is fine. Copilot, Claude, ChatGPT, whatever helps you. Sending code you don't understand is not, because then nobody understands it: you can't answer questions about it, and fixing it later falls on me.

## The rule

You are the author of everything you submit, however it was written. If you can't explain a change without asking the tool that wrote it, it isn't ready.

## In practice

- **Understand every line.** I may ask why something is there. "The AI did it that way" isn't an answer, and neither is a pasted reply from it.
- **Run it.** Start the game and look at what you changed, then run `cargo test --release --workspace` and `target/release/weft.exe --test all`. "Should work" means you didn't check. A fixed bug comes with a test.
- **Read the code around it first.** AI tools love writing a second version of something the engine already has, and that's the most common thing I end up rejecting. Use what exists.
- **Keep it small.** If the diff is too big for you to read carefully, it's too big for me as well. It's fine to have several features in one pr, but don't make it a 5000 lines pr when in reality you needed only a 100.
- **Write the description and comments yourself.** Short, or in imperfect English, is completely fine, and translators are fine too. A wall of generated text tells me nothing about your changes.
- **Say what you used AI for.** It costs you nothing and tells me where to look closer.
- **Check what it claims.** Functions that don't exist, numbers nobody measured, bugs nobody reproduced: an issue or a benchmark should be something you saw yourself.
- **Agents don't act alone.** Nothing gets opened or posted here by an agent before a person has read it.
- **It has to be yours to give.** The project is [MPL-2.0](LICENSE), and that goes for generated code too. No Mojang assets, generated look-alikes of them included.

## If a pull request doesn't follow this

I close it without a review. No hard feelings: fix it and open it again. If it keeps happening, I stop taking pull requests from that account.
