// SPDX-License-Identifier: GPL-2.0-or-later
#include "modaloverlay.h"

#include <QApplication>
#include <QCloseEvent>
#include <QContextMenuEvent>
#include <QGraphicsBlurEffect>
#include <QGraphicsPixmapItem>
#include <QGraphicsScene>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QMouseEvent>
#include <QPainter>
#include <QStyle>
#include <QToolButton>
#include <QVBoxLayout>
#include <QWheelEvent>

#include "base/global.h"

Content::ModalOverlay::ModalOverlay(QWidget *host)
    : QWidget(host)
{
    Q_ASSERT(host);
    setObjectName(u"contentModalOverlay"_s);
    setFocusPolicy(Qt::StrongFocus);
    hide();
    m_backdropRefresh.setSingleShot(true);
    connect(&m_backdropRefresh, &QTimer::timeout, this, [this]
    {
        if (isVisible())
        {
            captureBackdrop();
            update();
        }
    });

    m_panel = new QWidget(this);
    m_panel->setObjectName(u"contentModalPanel"_s);
    m_panel->setAutoFillBackground(true);
    auto *panelLayout = new QVBoxLayout(m_panel);
    panelLayout->setContentsMargins(24, 20, 24, 24);
    panelLayout->setSpacing(16);
    auto *header = new QHBoxLayout;
    m_title = new QLabel(m_panel);
    m_title->setObjectName(u"contentModalTitle"_s);
    m_title->setTextFormat(Qt::PlainText);
    m_title->setWordWrap(true);
    QFont font = m_title->font();
    font.setBold(true);
    font.setPointSize(font.pointSize() + 2);
    m_title->setFont(font);
    header->addWidget(m_title, 1);
    m_close = new QToolButton(m_panel);
    m_close->setObjectName(u"contentModalClose"_s);
    m_close->setAutoRaise(true);
    updateCloseIcon();
    m_close->setToolTip(tr("Close"));
    m_close->setAccessibleName(tr("Close"));
    header->addWidget(m_close, 0, Qt::AlignTop);
    panelLayout->addLayout(header);
    m_content = new QWidget(m_panel);
    panelLayout->addWidget(m_content, 1);
    connect(m_close, &QToolButton::clicked, this, &QWidget::close);
}

void Content::ModalOverlay::setTitle(const QString &title)
{
    m_title->setText(title);
    setAccessibleName(title);
}

QWidget *Content::ModalOverlay::contentWidget() const
{
    return m_content;
}

void Content::ModalOverlay::open(QWidget *initialFocus)
{
    if (isVisible())
        return;

    m_previousFocus = parentWidget()->focusWidget();
    setGeometry(parentWidget()->rect());
    captureBackdrop();
    raise();
    show();
    qApp->installEventFilter(this);
    if (initialFocus && isAncestorOf(initialFocus))
        initialFocus->setFocus(Qt::PopupFocusReason);
    else
        m_close->setFocus(Qt::PopupFocusReason);
}

bool Content::ModalOverlay::eventFilter(QObject *watched, QEvent *event)
{
    if ((watched == parentWidget()) && (event->type() == QEvent::Resize))
    {
        setGeometry(parentWidget()->rect());
        m_backdropRefresh.start();
    }
    if (!isVisible())
        return false;

    if ((event->type() == QEvent::KeyPress) || (event->type() == QEvent::ShortcutOverride))
    {
        auto *key = static_cast<QKeyEvent *>(event);
        if ((key->key() == Qt::Key_Escape) && (key->modifiers() == Qt::NoModifier)
            && parentWidget()->isActiveWindow())
        {
            key->accept();
            if (event->type() == QEvent::KeyPress)
                close();
            return true;
        }
    }

    if ((event->type() == QEvent::Shortcut) || (event->type() == QEvent::FocusIn))
    {
        QObject *owner = watched;
        while (owner && !qobject_cast<QWidget *>(owner))
            owner = owner->parent();
        auto *widget = qobject_cast<QWidget *>(owner);
        if (widget && ((widget == parentWidget()) || parentWidget()->isAncestorOf(widget))
            && (widget != this) && !isAncestorOf(widget))
        {
            if (event->type() == QEvent::FocusIn)
                m_close->setFocus(Qt::OtherFocusReason);
            return true;
        }
    }
    return false;
}

bool Content::ModalOverlay::focusNextPrevChild(const bool next)
{
    QWidget *const current = focusWidget() ? focusWidget() : this;
    QWidget *candidate = current;
    do
    {
        candidate = next ? candidate->nextInFocusChain() : candidate->previousInFocusChain();
        if (isAncestorOf(candidate) && candidate->isVisible() && candidate->isEnabled()
            && (candidate->focusPolicy() & Qt::TabFocus))
        {
            candidate->setFocus(next ? Qt::TabFocusReason : Qt::BacktabFocusReason);
            return true;
        }
    } while (candidate != current);
    return true;
}

void Content::ModalOverlay::changeEvent(QEvent *event)
{
    if ((event->type() == QEvent::PaletteChange) || (event->type() == QEvent::StyleChange))
    {
        updateCloseIcon();
        m_backdropRefresh.start();
    }
    QWidget::changeEvent(event);
}

void Content::ModalOverlay::closeEvent(QCloseEvent *event)
{
    qApp->removeEventFilter(this);
    m_backdropRefresh.stop();
    hide();
    m_backdrop = {};
    m_backdropPressed = false;
    if (m_previousFocus && m_previousFocus->isVisible() && m_previousFocus->isEnabled())
        m_previousFocus->setFocus(Qt::PopupFocusReason);
    m_previousFocus.clear();
    event->accept();
    emit closed();
}

void Content::ModalOverlay::contextMenuEvent(QContextMenuEvent *event)
{
    event->accept();
}

void Content::ModalOverlay::keyPressEvent(QKeyEvent *event)
{
    event->accept();
}

void Content::ModalOverlay::keyReleaseEvent(QKeyEvent *event)
{
    event->accept();
}

void Content::ModalOverlay::mousePressEvent(QMouseEvent *event)
{
    m_backdropPressed = (event->button() == Qt::LeftButton)
        && rect().contains(event->position().toPoint())
        && !m_panel->geometry().contains(event->position().toPoint());
    event->accept();
}

void Content::ModalOverlay::mouseReleaseEvent(QMouseEvent *event)
{
    const bool dismiss = m_backdropPressed && (event->button() == Qt::LeftButton)
        && rect().contains(event->position().toPoint())
        && !m_panel->geometry().contains(event->position().toPoint());
    m_backdropPressed = false;
    event->accept();
    if (dismiss)
        close();
}

void Content::ModalOverlay::paintEvent(QPaintEvent *)
{
    QPainter painter(this);
    painter.drawPixmap(rect(), m_backdrop);
    painter.fillRect(rect(), QColor(0, 0, 0, 112));
}

void Content::ModalOverlay::resizeEvent(QResizeEvent *)
{
    const QSize panelSize = (size() - QSize(64, 64)).boundedTo(QSize(720, 600));
    m_panel->setGeometry(QRect(QPoint((width() - panelSize.width()) / 2,
        (height() - panelSize.height()) / 2), panelSize));
}

void Content::ModalOverlay::wheelEvent(QWheelEvent *event)
{
    event->accept();
}

void Content::ModalOverlay::captureBackdrop()
{
    // Render the host without this overlay, keeping its live children visible and focused.
    QWidget *host = parentWidget();
    QPixmap snapshot(host->size() * host->devicePixelRatioF());
    snapshot.setDevicePixelRatio(host->devicePixelRatioF());
    snapshot.fill(Qt::transparent);
    {
        QPainter painter(&snapshot);
        host->render(&painter, QPoint(), QRegion(), QWidget::DrawWindowBackground);
        for (QObject *child : host->children())
        {
            auto *widget = qobject_cast<QWidget *>(child);
            if (widget && (widget != this) && widget->isVisible() && !widget->isWindow())
                widget->render(&painter, widget->pos());
        }
    }
    // Cache a reduced blur between opening, resize and appearance changes.
    snapshot = snapshot.scaled(host->size() / 2, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
    snapshot.setDevicePixelRatio(1);
    QGraphicsScene scene;
    auto *image = scene.addPixmap(snapshot);
    auto *blur = new QGraphicsBlurEffect;
    blur->setBlurRadius(8);
    blur->setBlurHints(QGraphicsBlurEffect::QualityHint);
    image->setGraphicsEffect(blur);
    m_backdrop = QPixmap(snapshot.size());
    m_backdrop.fill(Qt::transparent);
    QPainter painter(&m_backdrop);
    scene.render(&painter, QRectF(m_backdrop.rect()), QRectF(snapshot.rect()));
}

void Content::ModalOverlay::updateCloseIcon()
{
    QPixmap icon = style()->standardIcon(QStyle::SP_DialogCloseButton).pixmap(m_close->iconSize(), devicePixelRatioF());
    QPainter painter(&icon);
    painter.setCompositionMode(QPainter::CompositionMode_SourceIn);
    painter.fillRect(icon.rect(), palette().color(QPalette::ButtonText));
    painter.end();
    m_close->setIcon(QIcon(icon));
}
