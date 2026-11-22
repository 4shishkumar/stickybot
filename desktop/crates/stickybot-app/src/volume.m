// A custom view on the existing tray NSMenuItem. AppKit tracks it in the menu,
// including mouse-up outside the slider and keyboard/VoiceOver actions.
#import <AppKit/AppKit.h>

typedef void (^SBVolumeChange)(int on, int raw);

@interface SBVolumeSlider : NSSlider
@property(nonatomic) BOOL dragging;
@property(nonatomic, copy) void (^started)(void);
@property(nonatomic, copy) void (^finished)(void);
@end
@implementation SBVolumeSlider
- (void)mouseDown:(NSEvent *)event {
    self.dragging = YES;
    self.started();
    [super mouseDown:event];
    self.dragging = NO;
    self.finished();
}
@end

@interface SBVolumeView : NSView
@property(nonatomic, strong) SBVolumeSlider *slider;
@property(nonatomic, strong) NSButton *mute;
@property(nonatomic, strong) NSTextField *level;
@property(nonatomic, copy) SBVolumeChange change;
@property(nonatomic) int stored;
@property(nonatomic) int sent;
@property(nonatomic) BOOL muted;
@property(nonatomic, copy) NSString *device;
@property(nonatomic) BOOL ignoreGesture;
@property(nonatomic) NSTimeInterval lastSend;
@end

@implementation SBVolumeView
- (instancetype)init {
    self = [super initWithFrame:NSMakeRect(0, 0, 242, 53)];
    if (!self) return nil;
    NSTextField *title = [NSTextField labelWithString:@"Stick volume"];
    title.frame = NSMakeRect(15, 28, 146, 18);
    [self addSubview:title];
    _level = [NSTextField labelWithString:@"—"];
    _level.frame = NSMakeRect(177, 28, 50, 18);
    _level.alignment = NSTextAlignmentRight;
    [self addSubview:_level];
    _mute = [NSButton buttonWithTitle:@"Mute" target:self action:@selector(toggle:)];
    _mute.frame = NSMakeRect(11, 2, 76, 26);
    [self addSubview:_mute];
    _slider = [[SBVolumeSlider alloc] initWithFrame:NSMakeRect(91, 5, 138, 22)];
    _slider.minValue = 0;
    _slider.maxValue = 100;
    _slider.continuous = YES;
    _slider.target = self;
    _slider.action = @selector(slide:);
    [_slider setAccessibilityLabel:@"Stick volume"];
    __weak SBVolumeView *weakSelf = self;
    _slider.started = ^{ weakSelf.ignoreGesture = NO; };
    _slider.finished = ^{
        SBVolumeView *view = weakSelf;
        if (view.ignoreGesture) {
            view.slider.doubleValue = view.sent;
            [view display:view.sent];
            view.ignoreGesture = NO;
        } else {
            [view sendLevel];
        }
    };
    [self addSubview:_slider];
    return self;
}
- (void)display:(int)pct {
    self.level.stringValue = [NSString stringWithFormat:@"%d%%", pct];
    self.mute.title = pct ? @"Mute" : @"Unmute";
    self.muted = pct == 0;
}
- (void)sendLevel {
    int pct = (int)lround(self.slider.doubleValue);
    if (!self.change || pct == self.sent) return;
    self.sent = pct;
    self.lastSend = NSProcessInfo.processInfo.systemUptime;
    if (pct) self.stored = (pct * 255 + 50) / 100;
    self.change(pct != 0, self.stored);
}
- (void)slide:(id)sender {
    if (self.ignoreGesture) return;
    [self display:(int)lround(self.slider.doubleValue)];
    if (!self.slider.dragging || NSProcessInfo.processInfo.systemUptime - self.lastSend >= 0.20)
        [self sendLevel];
}
- (void)toggle:(id)sender {
    if (!self.change) return;
    BOOL on = self.muted;
    int raw = self.stored ? self.stored : 140; // legacy volume zero has no saved audible level
    self.sent = on ? MAX(1, (raw * 100 + 127) / 255) : 0;
    self.slider.doubleValue = self.sent;
    [self display:self.sent];
    self.change(on, raw);
}
@end

void stickybot_volume_update(void *nativeMenu, const char *device, int enabled, int on, int raw, SBVolumeChange change) {
    NSMenu *menu = (__bridge NSMenu *)nativeMenu;
    NSMenuItem *item = [menu itemWithTitle:@"Stick volume"];
    SBVolumeView *view = (SBVolumeView *)item.view;
    if (!view) { view = [SBVolumeView new]; item.view = view; }
    view.slider.enabled = view.mute.enabled = enabled;
    view.change = change;
    NSString *identifier = [NSString stringWithUTF8String:device];
    if (view.slider.dragging) {
        if (![identifier isEqualToString:view.device] || !enabled) view.ignoreGesture = YES;
        if (!view.ignoreGesture) return;
    }
    view.device = identifier;
    view.stored = raw;
    view.sent = on && raw ? MAX(1, (raw * 100 + 127) / 255) : 0;
    view.slider.doubleValue = view.sent;
    [view display:view.sent];
    if (!enabled) view.level.stringValue = @"—";
}
