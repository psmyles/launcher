TODO

Hand-checks needed on Windows: the installer and PATH box, the add-on finding the install folder, and the Open step. I didn't run Open on the Mac either, since it would launch the app.

Launcher-side gaps:

When an add-on's script stops with an error, the message is labelled with the tool's file name but the add-on's line number (e.g. check-models.launcher:214).

Each step carries its own copy of the CLI finder until the launcher can share code across one add-on's steps.
