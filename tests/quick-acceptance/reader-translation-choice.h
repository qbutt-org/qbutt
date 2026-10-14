// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

class QString;

namespace Content
{
    class ContentView;
}

void exerciseReaderTranslationChoiceRace(Content::ContentView *view, const QString &profileDir);

void exerciseReaderOpenRouterSettings(Content::ContentView *view, const QString &profileDir);
