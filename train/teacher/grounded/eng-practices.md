<!-- source: eng-practices/review/index.md; section: 02b7bd938709bdf9; licence: CC-BY -->
### User
I've been asked to review code for the first time. What should I actually be looking at?

### Assistant
Google's code review guide lists eight things, roughly from the biggest questions to the smallest:

- **Design.** Is the change well designed, and does it belong in this part of the system?
- **Functionality.** Does it do what the author intended, and is that good for the people who will use it?
- **Complexity.** Could it be simpler? Will the next developer understand it quickly?
- **Tests.** Are there correct, well-designed automated tests?
- **Naming.** Are variables, types and functions named clearly?
- **Comments.** Are they clear, and do they explain what the code can't?
- **Style.** Does it follow the project's style guide?
- **Documentation.** Did the author update the docs the change affects?

Start at the top. A design problem makes comments about naming pointless, because the code will change anyway.

### User
How do I pick a reviewer for my own changes?

### Assistant
Pick the person who can give the most thorough and correct review and can respond in reasonable time. Usually that is the owner of the code you are changing, who may or may not be listed in an OWNERS file. For a change that touches several areas, ask different people to review different parts. If the ideal reviewer is unavailable, CC them anyway so they see it.

Code written in pair programming with someone qualified to review it counts as reviewed. An in-person review also works: the reviewer asks questions, and the author answers only what is asked.

---

<!-- source: eng-practices/review/developer/small-cls.md; section: 04ef934bc05af6a4; licence: CC-BY -->
### User
How big should a pull request be?

### Assistant
One self-contained change. In practice that means:

- It does one thing, usually one part of a feature rather than the whole feature.
- It includes the tests for that thing.
- A reviewer can understand it from the change, its description, the existing code and changes they have already reviewed.
- The system still works for users and developers after it is merged.
- It is not so small that its purpose is unclear: a new API should come with a use of it, so the reviewer sees how it is meant to be used and no unused API gets merged.

There is no fixed limit, but as a rough guide 100 lines is usually reasonable and 1000 lines is usually too large. Spread matters too: 200 lines in one file can be fine, while the same 200 lines across 50 files usually is not.

When unsure, go smaller. You know the code from having written it; the reviewer often starts with no context, and what feels like a normal size to you can be overwhelming to them. Reviewers rarely complain that a change is too small.

---

<!-- source: eng-practices/review/reviewer/looking-for.md; section: 13f4373822662646; licence: CC-BY -->
### User
The PR I'm reviewing has tests and they pass. Is that enough for the testing part of the review?

### Assistant
No. Passing tests tell you the tests agree with the code, not that they are any good. Nobody writes tests for the tests, so the reviewer is the one who checks them. Ask:

- Would these tests fail if the code were broken? A test that passes against a wrong implementation protects nothing.
- Will they break for the wrong reasons when the code changes underneath them, giving false failures?
- Does each test make simple, useful assertions?
- Are different behaviours split into separate tests, so a failure points at one thing?

Also check that the tests came with the change: unit, integration or end-to-end tests as the change needs, in the same pull request, unless it is an emergency fix. And hold tests to the same standard as other code. They have to be read and maintained too, so complexity in a test is not acceptable just because it does not ship in the binary.
