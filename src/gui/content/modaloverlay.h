// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QPixmap>
#include <QPointer>
#include <QTimer>
#include <QWidget>

class QLabel;
class QToolButton;

namespace Content
{
    class ModalOverlay final : public QWidget
    {
        Q_OBJECT

    public:
        explicit ModalOverlay(QWidget *host);

        void setTitle(const QString &title);
        QWidget *contentWidget() const;
        void open(QWidget *initialFocus = nullptr);

    signals:
        void closed();

    private:
        bool eventFilter(QObject *watched, QEvent *event) override;
        bool focusNextPrevChild(bool next) override;
        void changeEvent(QEvent *event) override;
        void closeEvent(QCloseEvent *event) override;
        void contextMenuEvent(QContextMenuEvent *event) override;
        void keyPressEvent(QKeyEvent *event) override;
        void keyReleaseEvent(QKeyEvent *event) override;
        void mousePressEvent(QMouseEvent *event) override;
        void mouseReleaseEvent(QMouseEvent *event) override;
        void paintEvent(QPaintEvent *event) override;
        void resizeEvent(QResizeEvent *event) override;
        void wheelEvent(QWheelEvent *event) override;
        void captureBackdrop();
        void updateCloseIcon();

        QWidget *m_panel = nullptr;
        QWidget *m_content = nullptr;
        QLabel *m_title = nullptr;
        QToolButton *m_close = nullptr;
        QPointer<QWidget> m_previousFocus;
        QPixmap m_backdrop;
        QTimer m_backdropRefresh;
        bool m_backdropPressed = false;
    };
}
