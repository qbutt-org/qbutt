// SPDX-License-Identifier: GPL-2.0-or-later
#include "contentview.h"

#include <QEvent>
#include <QQmlEngine>
#include <QQuickItem>

#include "qbutt_quick/src/controller.cxxqt.h"

Content::ContentView::ContentView(const QString &profileDir, QWidget *parent)
    : QQuickWidget(parent)
    , m_controller {new Qbutt::ContentController}
{
    setObjectName(QStringLiteral("contentView"));
    m_controller->setParent(this);
    m_controller->initLibrary(profileDir);
    m_controller->setObjectName(QStringLiteral("contentController"));
    connect(m_controller, &Qbutt::ContentController::torrentReady, this, &ContentView::torrentReady);
    connect(m_controller, &Qbutt::ContentController::videoReady, this, &ContentView::videoReady);
    connect(m_controller, &Qbutt::ContentController::readerChanged, this, &ContentView::readerActiveChanged);
    connect(m_controller, &Qbutt::ContentController::searchStateChanged, this, [this]
    {
        const auto state = m_controller->getSearchState();
        if ((state == Qbutt::Search::SearchState::Succeeded) || (state == Qbutt::Search::SearchState::Failed))
            emit searchFinished(state == Qbutt::Search::SearchState::Failed);
    });

    setResizeMode(QQuickWidget::SizeRootObjectToView);
    setInitialProperties({{QStringLiteral("backend"), QVariant::fromValue(m_controller)}});
    loadFromModule(QStringLiteral("Qbutt.Views"), QStringLiteral("SearchPage"));
}

void Content::ContentView::giveFocusToSearchInput()
{
    if (!rootObject())
        return;

    if (auto *query = rootObject()->findChild<QQuickItem *>(QStringLiteral("contentQuery")))
    {
        query->forceActiveFocus(Qt::ShortcutFocusReason);
        QMetaObject::invokeMethod(query, "selectAll");
    }
}

bool Content::ContentView::isReading() const
{
    return m_controller->reader_open();
}

bool Content::ContentView::confirmPlayback(const quint64 launchToken)
{
    return m_controller->confirmPlayback(launchToken);
}

bool Content::ContentView::finishPlayback(const quint64 launchToken)
{
    return m_controller->finishPlayback(launchToken);
}

void Content::ContentView::changeEvent(QEvent *event)
{
    QQuickWidget::changeEvent(event);
    if (event->type() == QEvent::LanguageChange)
        engine()->retranslate();
}
