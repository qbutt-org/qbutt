// SPDX-License-Identifier: GPL-2.0-or-later
#include "browserprerequisitewidget.h"

#include <QLabel>
#include <QPushButton>
#include <QScrollArea>
#include <QVBoxLayout>

#include "base/global.h"
#include "gui/uithememanager.h"

#include "browserconnectionwidget.h"

Content::BrowserPrerequisiteWidget::BrowserPrerequisiteWidget(QWidget *parent)
    : QWidget(parent)
{
    setObjectName(u"contentBrowserPrerequisite"_s);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    auto *back = new QPushButton(UIThemeManager::instance()->getIcon(u"go-previous"_s), tr("Back"), this);
    back->setObjectName(u"contentBrowserBack"_s);
    layout->addWidget(back, 0, Qt::AlignLeft);
    auto *scroll = new QScrollArea(this);
    scroll->setFrameShape(QFrame::NoFrame);
    scroll->setWidgetResizable(true);
    auto *page = new QWidget(scroll);
    auto *pageLayout = new QVBoxLayout(page);
    pageLayout->setSizeConstraint(QLayout::SetMinimumSize);
    pageLayout->setContentsMargins(24, 24, 24, 24);
    pageLayout->addStretch();
    auto *body = new QWidget(page);
    body->setObjectName(u"contentBrowserPrerequisiteBody"_s);
    body->setMaximumWidth(640);
    auto *bodyLayout = new QVBoxLayout(body);
    bodyLayout->setSizeConstraint(QLayout::SetMinimumSize);
    bodyLayout->setContentsMargins(0, 0, 0, 0);
    bodyLayout->setSpacing(16);
    m_heading = new QLabel(body);
    QFont font = m_heading->font();
    font.setBold(true);
    font.setPointSize(font.pointSize() + 3);
    m_heading->setFont(font);
    m_heading->setTextFormat(Qt::PlainText);
    m_heading->setWordWrap(true);
    bodyLayout->addWidget(m_heading);
    auto *connection = new BrowserConnectionWidget(false, body);
    bodyLayout->addWidget(connection);
    pageLayout->addWidget(body);
    pageLayout->addStretch(2);
    scroll->setWidget(page);
    layout->addWidget(scroll, 1);
    connect(back, &QPushButton::clicked, this, &BrowserPrerequisiteWidget::backRequested);
    connect(connection, &BrowserConnectionWidget::connected, this, &BrowserPrerequisiteWidget::connected);
}

void Content::BrowserPrerequisiteWidget::setSource(const QString &name)
{
    m_heading->setText(tr("%1 needs the qcatch browser extension to open this video.").arg(name));
}
