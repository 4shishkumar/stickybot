// The Mac input for StickyBot. Carbon delivers only the registered chord, even
// when another app has focus. The audio engine exists only during an owned hold
// (or a hands-free recording, or a microphone test in Settings).
#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <Carbon/Carbon.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreMedia/CoreMedia.h>
#import <QuartzCore/QuartzCore.h>

// 1: a turn is reserved; 0: the agent is busy; 2: no stick is set up.
extern int sb_voice_begin(void);
extern void sb_voice_audio(const float *pcm, size_t count);
extern void sb_voice_cancel(void);
extern void sb_voice_cancel_turn(void);
extern void sb_voice_dashboard(void);

static const NSUInteger SBRate = 16000;
static const NSUInteger SBMaxSeconds = 60;
// a press shorter than this is a tap: it records nothing (two taps lock hands-free)
static const NSTimeInterval SBTapSeconds = .35;
static const NSTimeInterval SBDoubleTapSeconds = .5;
static const NSTimeInterval SBWarnSeconds = 50;
// the loudest 64 ms window must reach this level, or nothing is sent
static const float SBSilence = .04f;
static NSString *const SBMicDenied = @"Allow StickyBot in System Settings → Privacy & Security → Microphone, then try again.";
static NSString *const SBPrivacyURL = @"x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";

// ---- input devices ----------------------------------------------------------------------

static NSString *SBPreferredUID; // nil: follow the system default input
static BOOL SBSounds = YES;

static AudioDeviceID SBDefaultInput(void) {
    AudioObjectPropertyAddress address = {kAudioHardwarePropertyDefaultInputDevice,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    AudioDeviceID device = kAudioObjectUnknown;
    UInt32 size = sizeof(device);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, &device) != noErr)
        return kAudioObjectUnknown;
    return device;
}

static NSString *SBDeviceText(AudioDeviceID device, AudioObjectPropertySelector selector) {
    if (device == kAudioObjectUnknown) return nil;
    AudioObjectPropertyAddress address = {selector, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    CFStringRef value = NULL;
    UInt32 size = sizeof(value);
    if (AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &value) != noErr || !value) return nil;
    return CFBridgingRelease(value);
}

static BOOL SBHasInput(AudioDeviceID device) {
    AudioObjectPropertyAddress address = {kAudioDevicePropertyStreamConfiguration,
        kAudioDevicePropertyScopeInput, kAudioObjectPropertyElementMain};
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(device, &address, 0, NULL, &size) != noErr || !size) return NO;
    AudioBufferList *list = malloc(size);
    BOOL found = NO;
    if (list && AudioObjectGetPropertyData(device, &address, 0, NULL, &size, list) == noErr)
        for (UInt32 i = 0; i < list->mNumberBuffers; ++i)
            if (list->mBuffers[i].mNumberChannels) { found = YES; break; }
    free(list);
    return found;
}

static BOOL SBIsBluetooth(AudioDeviceID device) {
    AudioObjectPropertyAddress address = {kAudioDevicePropertyTransportType,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    UInt32 transport = 0, size = sizeof(transport);
    if (device == kAudioObjectUnknown ||
        AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &transport) != noErr) return NO;
    return transport == kAudioDeviceTransportTypeBluetooth || transport == kAudioDeviceTransportTypeBluetoothLE;
}

static NSArray<NSNumber *> *SBInputDevices(void) {
    AudioObjectPropertyAddress address = {kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &address, 0, NULL, &size) != noErr || !size) return @[];
    NSMutableData *ids = [NSMutableData dataWithLength:size];
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, ids.mutableBytes) != noErr) return @[];
    NSMutableArray *inputs = [NSMutableArray array];
    const AudioDeviceID *list = ids.bytes;
    for (UInt32 i = 0; i < size / sizeof(AudioDeviceID); ++i)
        // StickyBot's own virtual devices (the stick's wireless mic) are not a microphone to pick
        if (SBHasInput(list[i]) && ![SBDeviceText(list[i], kAudioDevicePropertyDeviceUID) hasPrefix:@"com.stickkey."])
            [inputs addObject:@(list[i])];
    return inputs;
}

// The chosen microphone when it is connected, else the system default.
static AudioDeviceID SBChosenInput(void) {
    if (SBPreferredUID.length)
        for (NSNumber *device in SBInputDevices())
            if ([SBDeviceText(device.unsignedIntValue, kAudioDevicePropertyDeviceUID) isEqual:SBPreferredUID])
                return device.unsignedIntValue;
    return SBDefaultInput();
}

static NSString *SBInputName(AudioDeviceID device) {
    return SBDeviceText(device, kAudioObjectPropertyName) ?: @"No input device";
}

// {"devices":[{uid,name,bluetooth}], "selected", "default_name", "using", "using_bluetooth", "fallback"}
const char *sb_voice_inputs(void) {
    static NSString *json;
    NSMutableArray *devices = [NSMutableArray array];
    for (NSNumber *n in SBInputDevices()) {
        AudioDeviceID device = n.unsignedIntValue;
        NSString *uid = SBDeviceText(device, kAudioDevicePropertyDeviceUID), *name = SBDeviceText(device, kAudioObjectPropertyName);
        if (uid && name) [devices addObject:@{@"uid": uid, @"name": name, @"bluetooth": @(SBIsBluetooth(device))}];
    }
    AudioDeviceID fallback = SBDefaultInput(), chosen = SBChosenInput();
    BOOL missing = SBPreferredUID.length && ![SBDeviceText(chosen, kAudioDevicePropertyDeviceUID) isEqual:SBPreferredUID];
    NSDictionary *out = @{@"devices": devices, @"selected": SBPreferredUID ?: @"",
        @"default_name": SBInputName(fallback), @"using": SBInputName(chosen),
        @"using_bluetooth": @(SBIsBluetooth(chosen)), @"fallback": @(missing)};
    NSData *data = [NSJSONSerialization dataWithJSONObject:out options:0 error:nil];
    json = data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : @"{}";
    return json.UTF8String;
}

void sb_voice_set_input(const char *uid) {
    NSString *value = uid ? [NSString stringWithUTF8String:uid] : nil;
    SBPreferredUID = value.length ? value : nil;
}

void sb_voice_set_sounds(int on) { SBSounds = on != 0; }

const char *sb_voice_permission(void) {
    switch ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]) {
        case AVAuthorizationStatusAuthorized: return "authorized";
        case AVAuthorizationStatusDenied: return "denied";
        case AVAuthorizationStatusRestricted: return "restricted";
        default: return "undetermined";
    }
}

static void SBPlay(NSString *name) {
    if (!SBSounds) return;
    NSSound *sound = [[NSSound soundNamed:name] copy];
    sound.volume = .35;
    [sound play];
}

// ---- recording --------------------------------------------------------------------------

// The system default input records through AVAudioEngine. A microphone picked in Settings records
// through AVCaptureSession, which opens a device by its UID (AVAudioEngine can't reliably switch
// its input away from the default) and delivers 16 kHz mono floats directly.
@interface SBRecording : NSObject <AVCaptureAudioDataOutputSampleBufferDelegate>
@property(nonatomic, readonly) AVAudioEngine *engine;
@property(nonatomic, readonly) AVCaptureSession *session;
@property(nonatomic, readonly) AudioDeviceID device;
@property(atomic, readonly) float level;
@property(atomic, readonly) float peak;
@property(atomic, copy, readonly) NSString *failure;
- (NSString *)start;
- (NSData *)finish;
- (NSTimeInterval)seconds;
- (BOOL)running;
@end

@implementation SBRecording {
    AVAudioConverter *_converter;
    NSMutableData *_samples;
    BOOL _tap, _closed;
}
@synthesize level = _level, peak = _peak, failure = _failure;
- (NSString *)start {
    _device = SBChosenInput();
    if (_device == kAudioObjectUnknown) return @"No microphone is available. Connect one, or choose another in StickyBot Settings.";
    _samples = [NSMutableData dataWithCapacity:SBMaxSeconds * SBRate * sizeof(float)];
    if (_device != SBDefaultInput()) return [self startCapture];
    NSError *error = nil;
    @try {
        _engine = [[AVAudioEngine alloc] init];
        AVAudioInputNode *input = _engine.inputNode;
        AVAudioFormat *source = [input outputFormatForBus:0];
        AVAudioFormat *mono = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:SBRate channels:1];
        if (!source.channelCount || source.sampleRate <= 0) return @"The selected microphone has no audio input.";
        _converter = [[AVAudioConverter alloc] initFromFormat:source toFormat:mono];
        if (!_converter) return @"The selected microphone format is unavailable.";
        _converter.downmix = YES;
        __weak SBRecording *weakSelf = self;
        AVAudioNodeTapBlock tap = ^(AVAudioPCMBuffer *buffer, AVAudioTime *__unused time) { [weakSelf append:buffer]; };
#if __MAC_OS_X_VERSION_MAX_ALLOWED >= 270000
        if (@available(macOS 27.0, *)) {
            if (![input installTapOnBus:0 bufferSize:1024 format:source error:&error block:tap])
                return error.localizedDescription ?: @"Could not open the microphone.";
        } else
#endif
        {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations" // available on the minimum macOS 13 target
            [input installTapOnBus:0 bufferSize:1024 format:source block:tap];
#pragma clang diagnostic pop
        }
        _tap = YES;
        if (![_engine startAndReturnError:&error])
            return [NSString stringWithFormat:@"Could not start %@. Another app may be using it.", SBInputName(_device)];
    } @catch (NSException *exception) {
        // Core Audio may withdraw the input between checking its format and installing the tap.
        return @"The microphone changed. Try again.";
    }
    return nil;
}

- (NSString *)startCapture {
    NSString *name = SBInputName(_device);
    AVCaptureDevice *device = [AVCaptureDevice deviceWithUniqueID:SBDeviceText(_device, kAudioDevicePropertyDeviceUID) ?: @""];
    if (!device) return [NSString stringWithFormat:@"Could not open %@. Choose another microphone in StickyBot Settings.", name];
    NSError *error = nil;
    AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
    AVCaptureAudioDataOutput *output = [AVCaptureAudioDataOutput new];
    output.audioSettings = @{AVFormatIDKey: @(kAudioFormatLinearPCM), AVSampleRateKey: @(SBRate), AVNumberOfChannelsKey: @1,
        AVLinearPCMBitDepthKey: @32, AVLinearPCMIsFloatKey: @YES, AVLinearPCMIsNonInterleaved: @NO, AVLinearPCMIsBigEndianKey: @NO};
    _session = [AVCaptureSession new];
    if (!input || ![_session canAddInput:input] || ![_session canAddOutput:output])
        return [NSString stringWithFormat:@"Could not open %@. Another app may be using it.", name];
    [_session addInput:input];
    [_session addOutput:output];
    [output setSampleBufferDelegate:self queue:dispatch_queue_create("stickybot.voice.capture", DISPATCH_QUEUE_SERIAL)];
    [_session startRunning];
    if (!_session.isRunning) return [NSString stringWithFormat:@"Could not start %@. Another app may be using it.", name];
    return nil;
}

- (void)captureOutput:(AVCaptureOutput *)output didOutputSampleBuffer:(CMSampleBufferRef)buffer fromConnection:(AVCaptureConnection *)connection {
    CMBlockBufferRef block = CMSampleBufferGetDataBuffer(buffer);
    size_t bytes = block ? CMBlockBufferGetDataLength(block) : 0;
    if (!bytes) return;
    NSMutableData *pcm = [NSMutableData dataWithLength:bytes];
    if (CMBlockBufferCopyDataBytes(block, 0, bytes, pcm.mutableBytes) != kCMBlockBufferNoErr) return;
    @synchronized (self) {
        if (_closed || _failure) return;
        [self appendMono:pcm.bytes count:bytes / sizeof(float)];
    }
}

// Called under @synchronized(self) with 16 kHz mono samples.
- (void)appendMono:(const float *)samples count:(NSUInteger)count {
    if (!count) return;
    if (_samples.length + count * sizeof(float) > SBMaxSeconds * SBRate * sizeof(float)) {
        _failure = @"Recording reached one minute.";
        return;
    }
    [_samples appendBytes:samples length:count * sizeof(float)];
    double power = 0;
    for (NSUInteger i = 0; i < count; ++i) power += samples[i] * samples[i];
    _level = fmin(1, sqrt(power / count) * 6);
    _peak = fmax(_peak, _level);
}

- (BOOL)running { return _session ? _session.isRunning : _engine.isRunning; }

- (void)append:(AVAudioPCMBuffer *)buffer {
    @synchronized (self) {
        if (_closed || _failure || !buffer.frameLength) return;
        if (![_converter.inputFormat isEqual:buffer.format]) {
            _failure = @"The microphone format changed. Try again.";
            return;
        }
        AVAudioFrameCount capacity = (AVAudioFrameCount)ceil(buffer.frameLength * (double)SBRate / buffer.format.sampleRate) + 64;
        AVAudioPCMBuffer *out = [[AVAudioPCMBuffer alloc] initWithPCMFormat:_converter.outputFormat frameCapacity:capacity];
        __block BOOL consumed = NO;
        NSError *error = nil;
        AVAudioConverterOutputStatus status = [_converter convertToBuffer:out error:&error
            withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount __unused packets, AVAudioConverterInputStatus *inputStatus) {
                if (consumed) { *inputStatus = AVAudioConverterInputStatus_NoDataNow; return nil; }
                consumed = YES;
                *inputStatus = AVAudioConverterInputStatus_HaveData;
                return buffer;
            }];
        if (status == AVAudioConverterOutputStatus_Error) {
            _failure = error.localizedDescription ?: @"The microphone audio could not be read.";
            return;
        }
        [self appendMono:out.floatChannelData[0] count:out.frameLength];
    }
}

// The tap runs on Core Audio's thread; the main run loop owns the engine.
- (float)level { @synchronized (self) { return _level; } }
- (float)peak { @synchronized (self) { return _peak; } }
- (NSString *)failure { @synchronized (self) { return _failure; } }
- (NSTimeInterval)seconds { @synchronized (self) { return _samples.length / (double)(SBRate * sizeof(float)); } }
- (NSData *)finish {
    @synchronized (self) { _closed = YES; }
    [_session stopRunning];
    [_engine stop];
    if (_tap) { [_engine.inputNode removeTapOnBus:0]; _tap = NO; }
    // The sample-rate converter retains the end of the last syllable. Drain its
    // filter after the tap stops, including when the user releases mid-buffer.
    if (_converter && !_failure) {
        AVAudioPCMBuffer *tail = [[AVAudioPCMBuffer alloc] initWithPCMFormat:_converter.outputFormat frameCapacity:1024];
        while (_samples.length < SBMaxSeconds * SBRate * sizeof(float)) {
            NSError *error = nil;
            AVAudioConverterOutputStatus status = [_converter convertToBuffer:tail error:&error
                withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount __unused n, AVAudioConverterInputStatus *s) {
                    *s = AVAudioConverterInputStatus_EndOfStream;
                    return nil;
                }];
            if (status == AVAudioConverterOutputStatus_Error) {
                _failure = error.localizedDescription ?: @"Could not finish reading the microphone.";
                break;
            }
            NSUInteger bytes = MIN(tail.frameLength * sizeof(float), SBMaxSeconds * SBRate * sizeof(float) - _samples.length);
            if (bytes) [_samples appendBytes:tail.floatChannelData[0] length:bytes];
            if (!bytes || status == AVAudioConverterOutputStatus_EndOfStream) break;
        }
    }
    return _samples;
}
@end

// ---- the floating panel -----------------------------------------------------------------

static NSColor *SBTeal(void) { return [NSColor colorWithSRGBRed:.35 green:.85 blue:.75 alpha:1]; }
static NSTextField *SBLabel(CGFloat size, NSFontWeight weight) {
    NSTextField *label = [NSTextField wrappingLabelWithString:@""];
    label.font = [NSFont systemFontOfSize:size weight:weight];
    label.textColor = NSColor.whiteColor;
    return label;
}

// Live microphone bars: the newest level on the right.
@interface SBWave : NSView
- (void)push:(float)level;
- (void)reset;
@end
@implementation SBWave {
    float _levels[9];
}
- (void)reset { memset(_levels, 0, sizeof(_levels)); self.needsDisplay = YES; }
- (void)push:(float)level {
    memmove(_levels, _levels + 1, sizeof(_levels) - sizeof(float));
    _levels[8] = level;
    self.needsDisplay = YES;
}
- (void)drawRect:(NSRect)dirty {
    NSRect b = self.bounds;
    CGFloat bar = 4, gap = (NSWidth(b) - 9 * bar) / 8;
    [SBTeal() setFill];
    for (int i = 0; i < 9; ++i) {
        CGFloat h = MAX(4, NSHeight(b) * fmin(1, _levels[i] * 1.4));
        NSRect r = NSMakeRect(i * (bar + gap), NSMidY(b) - h / 2, bar, h);
        [[NSBezierPath bezierPathWithRoundedRect:r xRadius:2 yRadius:2] fill];
    }
}
@end

typedef NS_ENUM(NSInteger, SBAction) { SBActionNone, SBActionDashboard, SBActionPrivacy };

@interface SBPanel : NSPanel
- (void)showPhase:(NSString *)phase heard:(NSString *)heard detail:(NSString *)detail;
- (void)setMicLevel:(float)level;
- (void)updateMotion;
- (void)hide;
// called whenever the panel leaves the screen (×, Esc, fading, a new recording's cancel…)
@property(nonatomic, copy) void (^onHidden)(void);
@end

@implementation SBPanel {
    NSImageView *_logo;
    NSTextField *_title, *_heard;
    NSButton *_cancel, *_action;
    SBWave *_wave;
    NSScrollView *_scroll;
    NSTextView *_text;
    BOOL _active;
    SBAction _actionKind;
    NSScreen *_screen;
    NSTimer *_fade;
}
- (instancetype)init {
    self = [super initWithContentRect:NSMakeRect(0, 0, 344, 100)
        styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
        backing:NSBackingStoreBuffered defer:NO];
    if (!self) return nil;
    self.level = NSFloatingWindowLevel;
    self.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorIgnoresCycle;
    self.becomesKeyOnlyIfNeeded = YES;
    self.hidesOnDeactivate = NO;
    self.releasedWhenClosed = NO;
    self.opaque = NO;
    self.backgroundColor = NSColor.clearColor;
    self.hasShadow = YES;
    self.title = @"StickyBot voice";
    NSView *v = self.contentView;
    v.wantsLayer = YES;
    v.layer.backgroundColor = [NSColor colorWithSRGBRed:.085 green:.115 blue:.12 alpha:.98].CGColor;
    v.layer.cornerRadius = 19;
    v.layer.borderWidth = 1;
    v.layer.borderColor = [NSColor colorWithWhite:.39 alpha:.65].CGColor;
    _logo = [NSImageView new];
    _logo.image = NSApp.applicationIconImage;
    _logo.imageScaling = NSImageScaleProportionallyUpOrDown;
    _logo.wantsLayer = YES;
    _logo.layer.shadowOffset = CGSizeZero;
    _logo.layer.shadowRadius = 6;
    _logo.layer.shadowColor = SBTeal().CGColor;
    _logo.accessibilityLabel = @"StickyBot";
    _title = SBLabel(13, NSFontWeightSemibold);
    _heard = SBLabel(12, NSFontWeightRegular);
    _heard.maximumNumberOfLines = 2;
    _heard.lineBreakMode = NSLineBreakByTruncatingTail;
    _heard.textColor = [NSColor colorWithWhite:.78 alpha:1];
    _cancel = [NSButton buttonWithImage:[NSImage imageWithSystemSymbolName:@"xmark" accessibilityDescription:@"Dismiss"]
                                target:self action:@selector(cancelOrDismiss:)];
    _cancel.bordered = NO;
    _cancel.contentTintColor = NSColor.lightGrayColor;
    _action = [NSButton buttonWithTitle:@"Open dashboard" target:self action:@selector(act:)];
    _action.bezelStyle = NSBezelStyleInline;
    _action.contentTintColor = NSColor.whiteColor;
    _wave = [SBWave new];
    _wave.accessibilityLabel = @"Microphone level";
    _scroll = [NSScrollView new];
    _scroll.drawsBackground = NO;
    _scroll.autohidesScrollers = YES;
    _scroll.scrollerStyle = NSScrollerStyleOverlay;
    _text = [[NSTextView alloc] initWithFrame:NSMakeRect(0, 0, 312, 20)];
    _text.drawsBackground = NO;
    _text.editable = NO;
    _text.selectable = NO;
    _text.font = [NSFont systemFontOfSize:13];
    _text.textColor = [NSColor colorWithWhite:.9 alpha:1];
    _text.textContainerInset = NSZeroSize;
    _text.textContainer.lineFragmentPadding = 0;
    _text.textContainer.widthTracksTextView = YES;
    _text.verticallyResizable = YES;
    _text.autoresizingMask = NSViewWidthSizable;
    _scroll.documentView = _text;
    for (NSView *item in @[_logo, _title, _heard, _cancel, _action, _wave, _scroll]) [v addSubview:item];
    return self;
}
- (BOOL)canBecomeKeyWindow { return NO; }
- (BOOL)canBecomeMainWindow { return NO; }
// While a request runs the button cancels it; afterwards it only hides the panel.
- (void)cancelOrDismiss:(id)sender {
    if (_active) sb_voice_cancel_turn();
    [self hide];
}
- (void)hide { [_fade invalidate]; _fade = nil; [self orderOut:nil]; }
- (void)orderOut:(id)sender {
    BOOL wasVisible = self.isVisible;
    [super orderOut:sender];
    if (wasVisible && self.onHidden) self.onHidden();
}
- (void)act:(id)sender {
    if (_actionKind == SBActionPrivacy) [NSWorkspace.sharedWorkspace openURL:[NSURL URLWithString:SBPrivacyURL]];
    else sb_voice_dashboard();
    if (_actionKind == SBActionPrivacy) [self hide];
}
- (void)setMicLevel:(float)level { [_wave push:level]; }
- (void)updateMotion {
    CALayer *layer = _logo.layer;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    layer.shadowOpacity = _active ? .62 : 0;
    [CATransaction commit];
    if (!_active || NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion) {
        [layer removeAnimationForKey:@"voice-pulse"];
    } else if (![layer animationForKey:@"voice-pulse"]) {
        CABasicAnimation *scale = [CABasicAnimation animationWithKeyPath:@"transform.scale"];
        scale.fromValue = @1;
        scale.toValue = @1.07;
        CABasicAnimation *glow = [CABasicAnimation animationWithKeyPath:@"shadowOpacity"];
        glow.fromValue = @.36;
        glow.toValue = @.82;
        CAAnimationGroup *pulse = [CAAnimationGroup animation];
        pulse.animations = @[scale, glow];
        pulse.autoreverses = YES;
        pulse.duration = .9;
        pulse.repeatCount = HUGE_VALF;
        pulse.timingFunction = [CAMediaTimingFunction functionWithName:kCAMediaTimingFunctionEaseInEaseOut];
        [layer addAnimation:pulse forKey:@"voice-pulse"];
    }
}
// Phases: listening, transcribing, thinking, answering, approval, reply, error, hint.
// `detail` is shown as given; the caller words listening hints (release / hands-free / time left).
- (void)showPhase:(NSString *)phase heard:(NSString *)heard detail:(NSString *)detail {
    NSDictionary *titles = @{@"listening":@"Listening", @"transcribing":@"Transcribing", @"thinking":@"Thinking",
        @"answering":@"Answering", @"approval":@"Waiting for approval", @"reply":@"StickyBot", @"error":@"Couldn't do that",
        @"hint":@"Hold to talk"};
    BOOL recording = [phase isEqual:@"listening"];
    BOOL complete = [phase isEqual:@"reply"];
    BOOL error = [phase isEqual:@"error"], hint = [phase isEqual:@"hint"];
    _active = !complete && !error && !hint;
    NSString *title = titles[phase] ?: @"StickyBot";
    BOOL announce = ![_title.stringValue isEqual:title] || !self.isVisible;
    _title.stringValue = title;
    if (!recording) [_wave reset];
    _heard.stringValue = heard.length ? [NSString stringWithFormat:@"“%@”", heard] : @"";
    BOOL pickScreen = recording || !self.isVisible || !_screen;
    NSArray<NSScreen *> *screens = NSScreen.screens;
    if (pickScreen || ![screens containsObject:_screen]) {
        _screen = NSScreen.mainScreen;
        for (NSScreen *s in screens) if (NSPointInRect(NSEvent.mouseLocation, s.frame)) { _screen = s; break; }
    }
    NSRect visible = _screen.visibleFrame;
    BOOL compact = recording || hint || [phase isEqual:@"transcribing"];
    CGFloat width = MIN(compact ? 300 : 360, NSWidth(visible) - 12);
    CGFloat bodyWidth = width - 32;
    _text.string = detail;
    _text.textContainer.containerSize = NSMakeSize(bodyWidth, CGFLOAT_MAX);
    [_text.layoutManager ensureLayoutForTextContainer:_text.textContainer];
    CGFloat fullHeight = detail.length ? ceil([_text.layoutManager usedRectForTextContainer:_text.textContainer].size.height) : 0;
    CGFloat textHeight = MIN(fullHeight, MIN(190, NSHeight(visible) / 3));
    BOOL longReply = complete && fullHeight > textHeight;
    _actionKind = longReply ? SBActionDashboard : (error && [detail isEqual:SBMicDenied]) ? SBActionPrivacy : SBActionNone;
    _action.title = _actionKind == SBActionPrivacy ? @"Open Privacy Settings" : @"Open dashboard";
    [_action sizeToFit];
    CGFloat footer = _actionKind != SBActionNone ? 28 : 0;
    CGFloat heardHeight = heard.length ? MIN(34, ceil([_heard.cell cellSizeForBounds:NSMakeRect(0,0,bodyWidth,60)].height)) + 4 : 0;
    CGFloat height = 66 + textHeight + heardHeight + footer;
    [self setFrame:NSMakeRect(round(NSMidX(visible) - width * .5), NSMinY(visible) + 14, width, height) display:YES];
    _logo.frame = NSMakeRect(11, height - 53, 46, 46);
    _title.frame = NSMakeRect(66, height - 39, width - 160, 19);
    _wave.hidden = !recording;
    _wave.frame = NSMakeRect(width - 92, height - 42, 60, 22);
    // nothing to cancel while the keys are held: releasing sends, Esc cancels
    _cancel.hidden = recording;
    _cancel.image = [NSImage imageWithSystemSymbolName:@"xmark" accessibilityDescription:_active ? @"Cancel" : @"Dismiss"];
    _cancel.accessibilityLabel = _active ? @"Cancel this request" : @"Dismiss voice panel";
    _cancel.toolTip = _active ? @"Cancel" : @"Dismiss";
    _cancel.frame = NSMakeRect(width - 36, height - 43, 26, 26);
    _heard.hidden = !heardHeight;
    _heard.frame = NSMakeRect(16, 12 + footer + textHeight, bodyWidth, heardHeight);
    _action.hidden = _actionKind == SBActionNone;
    _action.frame = NSMakeRect(13, 9, NSWidth(_action.frame) + 8, 21);
    _scroll.hasVerticalScroller = fullHeight > textHeight;
    _scroll.frame = NSMakeRect(16, 12 + footer, bodyWidth, textHeight);
    [_text setFrameSize:NSMakeSize(bodyWidth, MAX(textHeight, fullHeight))];
    [self updateMotion];
    // Errors and hints go away by themselves; an answer stays until dismissed or the next question.
    [_fade invalidate];
    _fade = nil;
    if ((error && _actionKind == SBActionNone) || hint)
        _fade = [NSTimer scheduledTimerWithTimeInterval:hint ? 2.5 : 6 target:self selector:@selector(hide) userInfo:nil repeats:NO];
    [self orderFrontRegardless];
    if (announce) NSAccessibilityPostNotificationWithUserInfo(self, NSAccessibilityAnnouncementRequestedNotification,
        @{NSAccessibilityAnnouncementKey:title, NSAccessibilityPriorityKey:@(NSAccessibilityPriorityMedium)});
}
@end

// ---- the shortcut -----------------------------------------------------------------------

static const OSType SBHotkeySignature = 'SbVc';
static const UInt32 SBEscapeID = 1;

@interface SBVoice : NSObject
- (NSString *)configure:(BOOL)enable code:(UInt32)code modifiers:(UInt32)mods label:(NSString *)label;
- (void)keyEvent:(UInt32)identifier pressed:(BOOL)pressed;
- (void)show:(NSString *)phase heard:(NSString *)heard detail:(NSString *)detail;
- (void)turnEnded;
- (NSString *)test:(BOOL)on;
- (float)testLevel;
- (void)shutdown;
@property(nonatomic, copy) NSString *configError;
@end

static OSStatus SBHotkeyEvent(EventHandlerCallRef __unused next, EventRef event, void *user) {
    EventHotKeyID key;
    if (GetEventParameter(event, kEventParamDirectObject, typeEventHotKeyID, NULL, sizeof(key), NULL, &key) != noErr ||
        key.signature != SBHotkeySignature) return eventNotHandledErr;
    [(__bridge SBVoice *)user keyEvent:key.id pressed:GetEventKind(event) == kEventHotKeyPressed];
    return noErr;
}
static NSEventModifierFlags SBFlags(UInt32 mods) {
    return ((mods & shiftKey) ? NSEventModifierFlagShift : 0) |
           ((mods & controlKey) ? NSEventModifierFlagControl : 0) |
           ((mods & cmdKey) ? NSEventModifierFlagCommand : 0) |
           ((mods & optionKey) ? NSEventModifierFlagOption : 0);
}

@implementation SBVoice {
    EventHotKeyRef _shortcut, _escape, _escapeHeld;
    NSTimer *_escapeTimer;
    EventHandlerRef _handler;
    UInt32 _code, _mods, _identifier;
    NSString *_label, *_shownDetail;
    BOOL _held, _pending, _requesting, _restorePanel;
    // hands-free: started by a double tap, sent by the next press
    BOOL _locked, _secondTap, _warned;
    NSTimeInterval _pressedAt, _tapAt;
    NSUInteger _suspended, _hints;
    SBPanel *_panel;
    SBRecording *_recording, *_test;
    NSTimer *_poll, *_hintTimer, *_testTimer;
    NSTimeInterval _deadline;
}
- (instancetype)init {
    self = [super init];
    if (!self) return nil;
    _identifier = SBEscapeID;
    _label = @"the shortcut";
    NSNotificationCenter *ws = NSWorkspace.sharedWorkspace.notificationCenter;
    for (NSString *name in @[NSWorkspaceWillSleepNotification, NSWorkspaceScreensDidSleepNotification, NSWorkspaceSessionDidResignActiveNotification])
        [ws addObserver:self selector:@selector(suspend:) name:name object:nil];
    for (NSString *name in @[NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification])
        [ws addObserver:self selector:@selector(resume:) name:name object:nil];
    [ws addObserver:self selector:@selector(motionChanged:) name:NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification object:nil];
    NSDistributedNotificationCenter *dc = NSDistributedNotificationCenter.defaultCenter;
    [dc addObserver:self selector:@selector(suspend:) name:@"com.apple.screenIsLocked" object:nil];
    [dc addObserver:self selector:@selector(resume:) name:@"com.apple.screenIsUnlocked" object:nil];
    return self;
}

- (NSString *)configure:(BOOL)enable code:(UInt32)code modifiers:(UInt32)mods label:(NSString *)label {
    if (!enable) {
        if (_shortcut) { UnregisterEventHotKey(_shortcut); _shortcut = NULL; }
        [self finishRecording:NO error:nil];
        _held = _locked = NO;
        self.configError = nil;
        return nil;
    }
    NSString *failed = [self registerCode:code modifiers:mods];
    if (failed) return failed;
    if (label.length) _label = [label copy];
    AVAuthorizationStatus auth = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    self.configError = (auth == AVAuthorizationStatusDenied || auth == AVAuthorizationStatusRestricted) ? SBMicDenied : nil;
    if (auth == AVAuthorizationStatusNotDetermined) [self requestPermission];
    return nil;
}

- (void)requestPermission {
    if (_requesting) return;
    _requesting = YES;
    __weak SBVoice *weakSelf = self;
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
        dispatch_async(dispatch_get_main_queue(), ^{
            SBVoice *voice = weakSelf;
            if (!voice) return;
            voice->_requesting = NO;
            if (!granted) {
                voice.configError = SBMicDenied;
                if (voice->_shortcut && !voice->_pending) [voice show:@"error" heard:@"" detail:SBMicDenied];
            }
        });
    }];
}

// Separated from permission prompting so binding conflicts can be exercised without opening the mic.
- (NSString *)registerCode:(UInt32)code modifiers:(UInt32)mods {
    if (!mods || (mods & ~(cmdKey | shiftKey | optionKey | controlKey)) || code > 127 || code == kVK_Escape)
        return @"Choose a key with Control, Option, Command or Shift.";
    if (!_handler) {
        EventTypeSpec events[] = {{kEventClassKeyboard,kEventHotKeyPressed}, {kEventClassKeyboard,kEventHotKeyReleased}};
        if (InstallEventHandler(GetApplicationEventTarget(), SBHotkeyEvent, 2, events, (__bridge void *)self, &_handler) != noErr)
            return @"macOS could not install the shortcut handler.";
    }
    if (!_shortcut || _code != code || _mods != mods) {
        // Acquire the replacement before releasing the working chord. Distinct IDs reject late key-ups.
        EventHotKeyID id = {SBHotkeySignature, _identifier + 1};
        EventHotKeyRef replacement = NULL;
        OSStatus status = RegisterEventHotKey(code, mods, id, GetApplicationEventTarget(), kEventHotKeyExclusive, &replacement);
        if (status != noErr) return @"This shortcut is unavailable. Choose another combination.";
        if (_shortcut) UnregisterEventHotKey(_shortcut);
        [self finishRecording:NO error:nil];
        _shortcut = replacement;
        _identifier = id.id;
        _code = code;
        _mods = mods;
        _held = _locked = NO;
    }
    return nil;
}

- (NSString *)listeningDetail {
    NSTimeInterval left = _deadline - NSProcessInfo.processInfo.systemUptime;
    NSString *send = _locked ? [NSString stringWithFormat:@"Press %@ to send", _label] : @"Release to send";
    if (left <= SBMaxSeconds - SBWarnSeconds) return [NSString stringWithFormat:@"%.0f s left · %@", MAX(0, ceil(left)), send];
    return [NSString stringWithFormat:@"%@%@ · Esc to cancel", _locked ? @"Hands-free · " : @"", send];
}

// Esc belongs to StickyBot only while its panel is up: it cancels a recording or a running
// request, and closes an answer or error. After an answer, Esc goes back to the front app after
// 15 s even if the panel stays (× still closes it).
- (void)armEscape:(BOOL)finished {
    EventHotKeyID escapeID = {SBHotkeySignature, SBEscapeID};
    if (!_escape && RegisterEventHotKey(kVK_Escape, 0, escapeID, GetApplicationEventTarget(), 0, &_escape) != noErr) _escape = NULL;
    // while the chord is held, Esc arrives with its modifiers (never Command+Option+Esc: Force Quit)
    if (_recording && !_escapeHeld && (_mods & (cmdKey | optionKey)) != (cmdKey | optionKey) &&
        RegisterEventHotKey(kVK_Escape, _mods, escapeID, GetApplicationEventTarget(), 0, &_escapeHeld) != noErr) _escapeHeld = NULL;
    [_escapeTimer invalidate];
    _escapeTimer = finished ? [NSTimer scheduledTimerWithTimeInterval:15 target:self selector:@selector(disarmEscape) userInfo:nil repeats:NO] : nil;
}
- (void)releaseHeldEscape {
    if (_escapeHeld) { UnregisterEventHotKey(_escapeHeld); _escapeHeld = NULL; }
}
- (void)disarmEscape {
    [_escapeTimer invalidate]; _escapeTimer = nil;
    [self releaseHeldEscape];
    if (_escape) { UnregisterEventHotKey(_escape); _escape = NULL; }
}

- (void)keyEvent:(UInt32)identifier pressed:(BOOL)pressed {
    if (identifier == SBEscapeID) {
        if (!pressed) return;
        if (_recording) { _locked = NO; [self finishRecording:NO error:nil]; return; }
        if (_pending) sb_voice_cancel_turn();
        [_panel hide];
        [self disarmEscape];
        return;
    }
    if (!_shortcut || identifier != _identifier) return;
    NSTimeInterval now = NSProcessInfo.processInfo.systemUptime;
    if (!pressed) {
        _held = NO;
        if (!_recording || _locked) return;
        if (now - _pressedAt < SBTapSeconds) {
            if (_secondTap) {
                // a double tap: keep listening until the next press
                _locked = YES;
                [self show:@"listening" heard:@"" detail:[self listeningDetail]];
                return;
            }
            // a single tap records nothing; say how to talk unless a second tap follows
            [self finishRecording:NO error:nil];
            _tapAt = now;
            [_hintTimer invalidate];
            _hintTimer = [NSTimer scheduledTimerWithTimeInterval:SBDoubleTapSeconds target:self selector:@selector(tapHint:) userInfo:nil repeats:NO];
            return;
        }
        [self finishRecording:YES error:nil];
        return;
    }
    if (_held || _suspended) return;
    _held = YES;
    if (_locked && _recording) { _locked = NO; [self finishRecording:YES error:nil]; return; }
    if (_pending) {
        // one request at a time: show where it is and how to cancel it
        if (!_panel.isVisible) [_panel orderFrontRegardless];
        return;
    }
    if (_recording) return;
    [_hintTimer invalidate]; _hintTimer = nil;
    _secondTap = now - _tapAt < SBDoubleTapSeconds;
    _tapAt = 0;
    _pressedAt = now;
    AVAuthorizationStatus permission = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    if (permission != AVAuthorizationStatusAuthorized) {
        if (permission == AVAuthorizationStatusNotDetermined) [self requestPermission];
        NSString *reason = permission == AVAuthorizationStatusNotDetermined ? @"Allow microphone access, then hold the shortcut again." : SBMicDenied;
        [self show:@"error" heard:@"" detail:reason];
        return;
    }
    self.configError = nil;
    [self stopTest];
    int begun = sb_voice_begin();
    if (begun != 1) {
        [self show:@"error" heard:@"" detail:begun == 2 ? @"Add a stick in StickyBot first."
                                                       : @"Still working on the last request. Try again when it finishes."];
        return;
    }
    _recording = [SBRecording new];
    [self armEscape:NO];
    NSString *error = [_recording start];
    if (error) { [self finishRecording:NO error:error]; return; }
    if (_recording.engine)
        [NSNotificationCenter.defaultCenter addObserver:self selector:@selector(inputChanged:) name:AVAudioEngineConfigurationChangeNotification object:_recording.engine];
    _deadline = now + SBMaxSeconds;
    _warned = NO;
    SBPlay(@"Tink");
    [self show:@"listening" heard:@"" detail:[self listeningDetail]];
    _poll = [NSTimer timerWithTimeInterval:.05 target:self selector:@selector(tick:) userInfo:nil repeats:YES];
    [NSRunLoop.mainRunLoop addTimer:_poll forMode:NSRunLoopCommonModes];
}

- (void)tapHint:(NSTimer *)timer {
    _hintTimer = nil;
    _tapAt = 0;
    if (_hints >= 3 || _recording || _pending) return;
    _hints++;
    [self show:@"hint" heard:@"" detail:[NSString stringWithFormat:@"Hold %@ while you speak, or double-tap it to talk hands-free.", _label]];
}

// A device that went away keeps what it already heard: a second or more is still sent.
- (void)lostInput:(NSString *)reason {
    BOOL keep = _recording.seconds >= 1 && !_recording.failure;
    _locked = NO;
    [self finishRecording:keep error:keep ? nil : reason];
}

- (void)tick:(NSTimer *)timer {
    if (!_recording) return;
    NSString *failure = _recording.failure;
    if (failure) { _locked = NO; [self finishRecording:_recording.seconds >= 1 error:nil]; return; }
    if (_recording.device != SBChosenInput() || !_recording.running) {
        [self lostInput:@"The microphone disconnected. Try again."];
        return;
    }
    NSTimeInterval now = NSProcessInfo.processInfo.systemUptime;
    if (now >= _deadline) { _locked = NO; [self finishRecording:YES error:nil]; return; }
    if (!_warned && now >= _deadline - (SBMaxSeconds - SBWarnSeconds)) {
        _warned = YES;
        SBPlay(@"Tink");
    }
    // Releasing a modifier first should stop capture even if the main key is still down.
    NSEventModifierFlags required = SBFlags(_mods);
    if (!_locked && (NSEvent.modifierFlags & required) != required) {
        _held = NO;
        if (now - _pressedAt < SBTapSeconds) return; // the key-up decides about taps
        [self finishRecording:YES error:nil];
        return;
    }
    [_panel setMicLevel:_recording.level];
    NSString *detail = [self listeningDetail];
    if (![detail isEqual:_shownDetail]) [self show:@"listening" heard:@"" detail:detail];
}

- (void)finishRecording:(BOOL)send error:(NSString *)reason {
    if (!_recording) return;
    [_poll invalidate]; _poll = nil;
    [self releaseHeldEscape];
    SBRecording *recording = _recording; _recording = nil;
    _locked = NO;
    if (recording.engine) [NSNotificationCenter.defaultCenter removeObserver:self name:AVAudioEngineConfigurationChangeNotification object:recording.engine];
    NSData *samples = [recording finish];
    NSString *failure = reason;
    if (send && !failure && samples.length && recording.peak < SBSilence)
        failure = [NSString stringWithFormat:@"Didn't hear anything from %@. Check the microphone in StickyBot Settings.", SBInputName(recording.device)];
    if (send && !failure && samples.length) {
        _pending = YES;
        SBPlay(@"Pop");
        [self show:@"transcribing" heard:@"" detail:@""];
        sb_voice_audio(samples.bytes, samples.length / sizeof(float));
    } else {
        sb_voice_cancel();
        if (send && !failure) failure = @"No audio captured. Hold the shortcut while you speak.";
        if (failure) [self show:@"error" heard:@"" detail:failure];
        else [_panel hide];
    }
}

- (void)show:(NSString *)phase heard:(NSString *)heard detail:(NSString *)detail {
    if (!_panel) {
        _panel = [SBPanel new];
        __weak SBVoice *weakSelf = self;
        _panel.onHidden = ^{ [weakSelf disarmEscape]; };
    }
    if ([phase isEqual:@"cancelled"]) {
        _restorePanel = NO;
        _pending = NO;
        [_panel hide];
        return;
    }
    _shownDetail = [phase isEqual:@"listening"] ? detail : nil;
    [_panel showPhase:phase heard:heard detail:detail];
    if (_suspended) { _restorePanel = YES; [_panel orderOut:nil]; return; }
    [self armEscape:[phase isEqual:@"reply"] || [phase isEqual:@"error"] || [phase isEqual:@"hint"]];
}
// The Rust turn finished (answered, failed, cancelled or panicked): the next press may start one.
- (void)turnEnded { _pending = NO; }
- (void)inputChanged:(NSNotification *)note {
    // AVAudioEngine posts from its own dispatch queue. Check the original engine on main.
    dispatch_async(dispatch_get_main_queue(), ^{
        if (note.object == self->_recording.engine) [self lostInput:@"The microphone changed. Try again."];
    });
}

// Settings' microphone test: open the chosen input for a few seconds and report its level.
- (NSString *)test:(BOOL)on {
    [self stopTest];
    if (!on) return nil;
    if (_recording) return @"StickyBot is listening right now.";
    AVAuthorizationStatus permission = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    if (permission == AVAuthorizationStatusNotDetermined) { [self requestPermission]; return @"Allow microphone access, then test again."; }
    if (permission != AVAuthorizationStatusAuthorized) return SBMicDenied;
    _test = [SBRecording new];
    NSString *error = [_test start];
    if (error) { [_test finish]; _test = nil; return error; }
    _testTimer = [NSTimer scheduledTimerWithTimeInterval:6 target:self selector:@selector(stopTest) userInfo:nil repeats:NO];
    return nil;
}
- (void)stopTest {
    [_testTimer invalidate]; _testTimer = nil;
    if (_test) { [_test finish]; _test = nil; }
}
// -1 when no test is running
- (float)testLevel { return _test ? (_test.running ? _test.level : 0) : -1; }

// Waking a display does not unlock it. Match each notification with its own resume.
- (NSUInteger)suspensionReason:(NSString *)name {
    if ([name isEqual:NSWorkspaceWillSleepNotification] || [name isEqual:NSWorkspaceDidWakeNotification]) return 1;
    if ([name isEqual:NSWorkspaceScreensDidSleepNotification] || [name isEqual:NSWorkspaceScreensDidWakeNotification]) return 2;
    if ([name isEqual:NSWorkspaceSessionDidResignActiveNotification] || [name isEqual:NSWorkspaceSessionDidBecomeActiveNotification]) return 4;
    return 8;
}
- (void)suspend:(NSNotification *)note {
    if (!NSThread.isMainThread) { dispatch_async(dispatch_get_main_queue(), ^{ [self suspend:note]; }); return; }
    _suspended |= [self suspensionReason:note.name];
    _held = NO;
    [self finishRecording:NO error:nil];
    [self stopTest];
    if (_panel.isVisible) _restorePanel = YES;
    [_panel orderOut:nil];
}
- (void)resume:(NSNotification *)note {
    if (!NSThread.isMainThread) { dispatch_async(dispatch_get_main_queue(), ^{ [self resume:note]; }); return; }
    _suspended &= ~[self suspensionReason:note.name];
    if (!_suspended && _restorePanel) { _restorePanel = NO; [_panel orderFrontRegardless]; }
}
- (void)motionChanged:(NSNotification *)note {
    dispatch_async(dispatch_get_main_queue(), ^{ [self->_panel updateMotion]; });
}
- (void)shutdown {
    [self configure:NO code:0 modifiers:0 label:nil];
    [self stopTest];
    [_hintTimer invalidate]; _hintTimer = nil;
    if (_handler) { RemoveEventHandler(_handler); _handler = NULL; }
    [NSWorkspace.sharedWorkspace.notificationCenter removeObserver:self];
    [NSDistributedNotificationCenter.defaultCenter removeObserver:self];
    [_panel hide];
}
@end

// All public entry points run on the AppKit main thread. UTF-8 C string results
// are borrowed; Rust copies them before returning to the main run loop.
static SBVoice *voice;
static SBVoice *SBVoiceController(void) {
    if (!voice) voice = [SBVoice new];
    return voice;
}
const char *sb_voice_configure(int enabled, uint32_t keycode, uint32_t carbon_mods, const char *label) {
    if (!voice && !enabled) return NULL;
    static NSString *configurationResult;
    configurationResult = [SBVoiceController() configure:enabled != 0 code:keycode modifiers:carbon_mods
                                                    label:label ? [NSString stringWithUTF8String:label] : nil];
    return configurationResult.UTF8String;
}
const char *sb_voice_config_error(void) {
    return voice.configError.UTF8String;
}
void sb_voice_panel(const char *phase, const char *heard, const char *detail) {
    [voice show:[NSString stringWithUTF8String:phase] heard:[NSString stringWithUTF8String:heard] detail:[NSString stringWithUTF8String:detail]];
}
void sb_voice_turn_ended(void) { [voice turnEnded]; }
const char *sb_voice_test(int on) {
    static NSString *testResult;
    testResult = [SBVoiceController() test:on != 0];
    return testResult.UTF8String;
}
float sb_voice_test_level(void) { return voice ? [voice testLevel] : -1; }
void sb_voice_shutdown(void) {
    [voice shutdown];
    voice = nil;
}
