// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

namespace Content
{
    class ContentView;
}

class QString;

void exerciseSyosetuReader(Content::ContentView *view, bool imageOnly);
void exerciseEstarReader(Content::ContentView *view);
void exercisePublicTextReader(Content::ContentView *view, const QString &profileDir,
    const QString &sourceId, bool reopenOnly);
